use std::collections::{BTreeMap, HashMap, HashSet};

use rusqlite::{params, Connection, OptionalExtension};
use serde::{Deserialize, Serialize};

use super::{digest_valid, hex_valid, Manifest, MAX_CLOCK};
use crate::{audio::PairSettings, library::LibraryResult};

#[derive(Clone, Deserialize, PartialEq, Serialize)]
#[serde(tag = "type", rename_all = "camelCase")]
enum Value {
    Name {
        name: String,
    },
    Mix {
        enabled: bool,
    },
    Deleted,
    Track {
        digest: String,
        position: Option<i64>,
    },
    Transition {
        outgoing: String,
        incoming: String,
        settings: Option<PairSettings>,
    },
}

impl Value {
    fn field(&self) -> String {
        match self {
            Self::Name { .. } => "name".into(),
            Self::Mix { .. } => "mix".into(),
            Self::Deleted => "deleted".into(),
            Self::Track { digest, .. } => format!("track/{digest}"),
            Self::Transition {
                outgoing, incoming, ..
            } => format!("transition/{outgoing}/{incoming}"),
        }
    }

    fn removed(&self) -> Option<Self> {
        match self {
            Self::Track { digest, .. } => Some(Self::Track {
                digest: digest.clone(),
                position: None,
            }),
            Self::Transition {
                outgoing, incoming, ..
            } => Some(Self::Transition {
                outgoing: outgoing.clone(),
                incoming: incoming.clone(),
                settings: None,
            }),
            _ => None,
        }
    }

    fn valid(&self) -> bool {
        match self {
            Self::Name { name } => !name.trim().is_empty() && name.chars().count() <= 120,
            Self::Mix { .. } | Self::Deleted => true,
            Self::Track { digest, position } => {
                digest_valid(digest)
                    && position.is_none_or(|position| (0..MAX_CLOCK).contains(&position))
            }
            Self::Transition {
                outgoing, incoming, ..
            } => outgoing != incoming && digest_valid(outgoing) && digest_valid(incoming),
        }
    }
}

#[derive(Deserialize, Serialize)]
pub(super) struct Register {
    playlist: String,
    clock: i64,
    value: Value,
}

impl Register {
    pub(super) fn in_scope(&self, included: &impl Fn(&str) -> bool) -> bool {
        match &self.value {
            Value::Track { digest, .. } => included(digest),
            Value::Transition {
                outgoing, incoming, ..
            } => included(outgoing) && included(incoming),
            _ => true,
        }
    }

    pub(super) fn into_key(self) -> (String, String) {
        (self.playlist, self.value.field())
    }

    pub(super) fn removal_key(&self) -> Option<(String, String)> {
        match self.value {
            Value::Track { position: None, .. } | Value::Transition { settings: None, .. } => {
                Some((self.playlist.clone(), self.value.field()))
            }
            _ => None,
        }
    }

    pub(super) fn valid(&self) -> bool {
        hex_valid(&self.playlist, 32) && (1..MAX_CLOCK).contains(&self.clock) && self.value.valid()
    }
}

type Fields = BTreeMap<String, Value>;

struct Identities {
    by_id: HashMap<i64, String>,
    active: HashMap<String, i64>,
}

impl Identities {
    fn read(db: &Connection) -> rusqlite::Result<Self> {
        let mut statement = db.prepare(
            "SELECT t.id, identity.digest, v.audio_id IS NULL AND EXISTS
               (SELECT 1 FROM source WHERE track_id=t.id)
             FROM track t LEFT JOIN track_variant v ON v.variant_id=t.id
             JOIN track a ON a.id=COALESCE(v.audio_id,t.id)
             JOIN sync_identity identity ON identity.track_id=a.id
             ORDER BY EXISTS (SELECT 1 FROM sync_import WHERE track_id=t.id), t.id",
        )?;
        let mut ids = Self {
            by_id: HashMap::new(),
            active: HashMap::new(),
        };
        let mut rows = statement.query([])?;
        while let Some(row) = rows.next()? {
            let (id, digest, active): (i64, String, bool) = (row.get(0)?, row.get(1)?, row.get(2)?);
            if active {
                ids.active.entry(digest.clone()).or_insert(id);
            }
            ids.by_id.insert(id, digest);
        }
        Ok(ids)
    }
}

struct Snapshot {
    fields: Fields,
    tracks: Vec<i64>,
    transitions: BTreeMap<(i64, i64), PairSettings>,
}

impl Snapshot {
    fn read(db: &Connection, id: i64, identities: &Identities) -> LibraryResult<Option<Self>> {
        let playlist = db
            .query_row(
                "SELECT name, EXISTS (SELECT 1 FROM playlist_mix WHERE playlist_id=playlist.id)
             FROM playlist WHERE id=?1",
                [id],
                |row| Ok((row.get::<_, String>(0)?, row.get::<_, bool>(1)?)),
            )
            .optional()?;
        let Some((name, enabled)) = playlist else {
            return Ok(None);
        };
        let mut fields = Fields::new();
        for value in [Value::Name { name }, Value::Mix { enabled }] {
            fields.insert(value.field(), value);
        }
        let mut query = db.prepare_cached(
            "SELECT track_id FROM playlist_track WHERE playlist_id=?1 ORDER BY position,track_id",
        )?;
        let tracks = query
            .query_map([id], |row| row.get(0))?
            .collect::<rusqlite::Result<Vec<_>>>()?;
        let mut seen = HashSet::new();
        for digest in tracks
            .iter()
            .filter_map(|local| identities.by_id.get(local))
        {
            if seen.insert(digest) {
                let value = Value::Track {
                    digest: digest.clone(),
                    position: Some((seen.len() - 1) as i64),
                };
                fields.insert(value.field(), value);
            }
        }
        let mut query = db.prepare_cached(
            "SELECT outgoing_id,incoming_id,settings FROM playlist_transition
             WHERE playlist_id=?1 ORDER BY outgoing_id,incoming_id",
        )?;
        let mut rows = query.query([id])?;
        let mut transitions = BTreeMap::new();
        while let Some(row) = rows.next()? {
            let (out, into, json): (i64, i64, String) = (row.get(0)?, row.get(1)?, row.get(2)?);
            let settings = serde_json::from_str::<PairSettings>(&json)?.validated();
            transitions.insert((out, into), settings);
            if let (Some(outgoing), Some(incoming)) =
                (identities.by_id.get(&out), identities.by_id.get(&into))
            {
                if outgoing != incoming {
                    let value = Value::Transition {
                        outgoing: outgoing.clone(),
                        incoming: incoming.clone(),
                        settings: Some(settings),
                    };
                    fields.entry(value.field()).or_insert(value);
                }
            }
        }
        Ok(Some(Self {
            fields,
            tracks,
            transitions,
        }))
    }
}

fn store(
    db: &Connection,
    playlist: &str,
    device: &str,
    clock: i64,
    value: &Value,
) -> LibraryResult<()> {
    db.prepare_cached(
        "INSERT INTO sync_register VALUES (?1,?2,?3,?4,?5)
         ON CONFLICT(playlist,field) DO UPDATE SET clock=excluded.clock,device=excluded.device,value=excluded.value
         WHERE (excluded.clock,excluded.device)>(clock,device)",
    )?.execute(params![playlist, value.field(), clock, device, serde_json::to_string(value)?])?;
    Ok(())
}

struct Mapping {
    key: String,
    id: Option<i64>,
    baseline: String,
}

fn mappings(db: &Connection) -> rusqlite::Result<Vec<Mapping>> {
    let mut query = db.prepare("SELECT key,local_id,baseline FROM sync_playlist ORDER BY key")?;
    let rows = query.query_map([], |row| {
        Ok(Mapping {
            key: row.get(0)?,
            id: row.get(1)?,
            baseline: row.get(2)?,
        })
    })?;
    rows.collect()
}

fn local_changes(
    db: &Connection,
    device: &str,
    clock: i64,
    identities: &Identities,
) -> LibraryResult<()> {
    db.execute(
        "INSERT INTO sync_playlist (key,local_id)
         SELECT lower(hex(randomblob(16))),id FROM playlist
         WHERE NOT EXISTS (SELECT 1 FROM sync_playlist WHERE local_id=playlist.id)",
        [],
    )?;
    for mapping in mappings(db)? {
        let Some(id) = mapping.id else { continue };
        let Some(snapshot) = Snapshot::read(db, id, identities)? else {
            store(db, &mapping.key, device, clock, &Value::Deleted)?;
            db.execute(
                "UPDATE sync_playlist SET local_id=NULL,baseline='{}' WHERE key=?1",
                [&mapping.key],
            )?;
            continue;
        };
        let baseline: Fields = serde_json::from_str(&mapping.baseline)?;
        let before_append = snapshot
            .fields
            .iter()
            .filter_map(|(field, value)| match value {
                Value::Track { position, .. } if baseline.contains_key(field) => *position,
                _ => None,
            })
            .max();
        let mut appended = Vec::new();
        for (field, value) in &snapshot.fields {
            if baseline.get(field) == Some(value) {
                continue;
            }
            if let Value::Track {
                digest,
                position: Some(position),
            } = value
            {
                if !baseline.contains_key(field)
                    && before_append.is_none_or(|last| *position > last)
                {
                    appended.push((*position, digest));
                    continue;
                }
            }
            store(db, &mapping.key, device, clock, value)?;
        }
        if !appended.is_empty() {
            // Append after unresolved members too. Their tracks may arrive in a later sync.
            let mut last: i64 = db.query_row(
                "SELECT COALESCE(MAX(json_extract(value,'$.position')),-1)
                 FROM sync_register WHERE playlist=?1 AND field LIKE 'track/%'",
                [&mapping.key],
                |row| row.get(0),
            )?;
            appended.sort_unstable();
            for (position, digest) in appended {
                last = (last + 1).max(position).min(MAX_CLOCK - 1);
                store(
                    db,
                    &mapping.key,
                    device,
                    clock,
                    &Value::Track {
                        digest: digest.clone(),
                        position: Some(last),
                    },
                )?;
            }
        }
        for (field, value) in baseline {
            if !snapshot.fields.contains_key(&field) {
                if let Some(removed) = value.removed() {
                    store(db, &mapping.key, device, clock, &removed)?;
                }
            }
        }
    }
    Ok(())
}

fn materialize(db: &Connection, mapping: &Mapping, identities: &Identities) -> LibraryResult<bool> {
    let mut query =
        db.prepare_cached("SELECT value FROM sync_register WHERE playlist=?1 ORDER BY field")?;
    let mut rows = query.query([&mapping.key])?;
    let mut name = None;
    let mut mix = false;
    let mut deleted = false;
    let mut entries = Vec::new();
    let mut pairs = Vec::new();
    while let Some(row) = rows.next()? {
        match serde_json::from_str::<Value>(&row.get::<_, String>(0)?)? {
            Value::Name { name: value } => name = Some(value),
            Value::Mix { enabled } => mix = enabled,
            Value::Deleted => deleted = true,
            Value::Track {
                digest,
                position: Some(position),
            } => entries.push((position, digest)),
            Value::Transition {
                outgoing,
                incoming,
                settings: Some(settings),
            } => {
                pairs.push((outgoing, incoming, settings.validated()));
            }
            _ => {}
        }
    }
    if deleted {
        db.execute("UPDATE sync_playlist SET local_id=NULL,baseline='{}' WHERE key=?1 AND local_id IS NOT NULL", [&mapping.key])?;
        return match mapping.id {
            Some(id) => Ok(db.execute("DELETE FROM playlist WHERE id=?1", [id])? > 0),
            None => Ok(false),
        };
    }
    let Some(name) = name else { return Ok(false) };
    let (id, created) = match mapping.id {
        Some(id) => (id, false),
        None => {
            let id = db.query_row(
                "INSERT INTO playlist (name) VALUES (?1) RETURNING id",
                [&name],
                |row| row.get(0),
            )?;
            db.execute(
                "UPDATE sync_playlist SET local_id=?2 WHERE key=?1",
                params![mapping.key, id],
            )?;
            (id, true)
        }
    };

    let previous = Snapshot::read(db, id, identities)?.ok_or("Missing local playlist")?;
    let tracks_by_digest: HashMap<_, _> = previous
        .tracks
        .iter()
        .filter_map(|id| {
            identities
                .by_id
                .get(id)
                .map(|digest| (digest.as_str(), *id))
        })
        .collect();
    entries.sort_unstable();
    let known = entries.into_iter().filter_map(|(_, digest)| {
        tracks_by_digest
            .get(digest.as_str())
            .filter(|&&id| {
                identities
                    .active
                    .get(&digest)
                    .is_none_or(|active| id == *active)
            })
            .or_else(|| identities.active.get(&digest))
            .copied()
    });
    let mut unknown = previous
        .tracks
        .iter()
        .enumerate()
        .filter(|(_, id)| !identities.by_id.contains_key(id))
        .peekable();
    let mut tracks = Vec::new();
    for id in known {
        while unknown
            .peek()
            .is_some_and(|&(position, _)| position <= tracks.len())
        {
            if let Some((_, local)) = unknown.next() {
                tracks.push(*local);
            }
        }
        tracks.push(id);
    }
    tracks.extend(unknown.map(|(_, id)| *id));
    let available: HashMap<_, _> = tracks
        .iter()
        .filter_map(|id| {
            identities
                .by_id
                .get(id)
                .map(|digest| (digest.as_str(), *id))
        })
        .collect();

    let mut transitions = BTreeMap::new();
    for (outgoing, incoming, settings) in pairs {
        if let (Some(&out), Some(&into)) = (
            available.get(outgoing.as_str()),
            available.get(incoming.as_str()),
        ) {
            transitions.insert((out, into), settings);
        }
    }
    let retained: HashSet<_> = tracks.iter().copied().collect();
    transitions.extend(
        previous
            .transitions
            .iter()
            .filter_map(|(&(a, b), &settings)| {
                ((!identities.by_id.contains_key(&a) || !identities.by_id.contains_key(&b))
                    && retained.contains(&a)
                    && retained.contains(&b))
                .then_some(((a, b), settings))
            }),
    );

    let mut changed = created;
    if previous.fields.get("name") != Some(&Value::Name { name: name.clone() }) {
        db.execute("UPDATE playlist SET name=?2 WHERE id=?1", params![id, name])?;
        changed = true;
    }
    if previous.fields.get("mix") != Some(&Value::Mix { enabled: mix }) {
        if mix {
            db.execute("INSERT OR IGNORE INTO playlist_mix VALUES (?1)", [id])?;
        } else {
            db.execute("DELETE FROM playlist_mix WHERE playlist_id=?1", [id])?;
        }
        changed = true;
    }
    if previous.tracks != tracks {
        db.execute("DELETE FROM playlist_track WHERE playlist_id=?1", [id])?;
        let mut insert = db.prepare_cached("INSERT INTO playlist_track VALUES (?1,?2,?3)")?;
        for (position, track) in tracks.iter().enumerate() {
            insert.execute(params![id, track, position as i64])?;
        }
        changed = true;
    }
    if previous.transitions != transitions {
        db.execute("DELETE FROM playlist_transition WHERE playlist_id=?1", [id])?;
        let mut insert =
            db.prepare_cached("INSERT INTO playlist_transition VALUES (?1,?2,?3,?4)")?;
        for ((out, into), settings) in transitions {
            insert.execute(params![id, out, into, serde_json::to_string(&settings)?])?;
        }
        changed = true;
    }
    let current = if changed {
        Snapshot::read(db, id, identities)?
            .ok_or("Missing applied playlist")?
            .fields
    } else {
        previous.fields
    };
    let baseline = serde_json::to_string(&current)?;
    if baseline != mapping.baseline {
        db.execute(
            "UPDATE sync_playlist SET baseline=?2 WHERE key=?1",
            params![mapping.key, baseline],
        )?;
    }
    Ok(changed)
}

pub(super) fn merge(
    db: &Connection,
    device: &str,
    peers: &[Manifest],
) -> LibraryResult<(Vec<Register>, usize)> {
    let identities = Identities::read(db)?;
    let clock: i64 = db.query_row("SELECT clock FROM sync_state", [], |row| row.get(0))?;
    let next = clock
        .checked_add(1)
        .filter(|&next| next < MAX_CLOCK)
        .ok_or("Playlist revision limit reached")?;
    // Local edits were based on the previous merge, so they share a clock below any
    // newer revisions from devices we have yet to see.
    local_changes(db, device, next, &identities)?;
    let mut latest = clock;
    for peer in peers {
        for register in &peer.playlists {
            store(
                db,
                &register.playlist,
                &peer.device,
                register.clock,
                &register.value,
            )?;
            latest = latest.max(register.clock);
        }
    }
    db.execute(
        "INSERT OR IGNORE INTO sync_playlist(key) SELECT DISTINCT playlist FROM sync_register",
        [],
    )?;
    let mut changed = 0;
    for mapping in mappings(db)? {
        changed += usize::from(materialize(db, &mapping, &identities)?);
    }
    let mut statement = db.prepare(
        "SELECT playlist,clock,value FROM sync_register WHERE device=?1 ORDER BY playlist,field",
    )?;
    let mut rows = statement.query([device])?;
    let mut outgoing = Vec::new();
    while let Some(row) = rows.next()? {
        let clock: i64 = row.get(1)?;
        latest = latest.max(clock);
        outgoing.push(Register {
            playlist: row.get(0)?,
            clock,
            value: serde_json::from_str(&row.get::<_, String>(2)?)?,
        });
    }
    if latest != clock {
        db.execute("UPDATE sync_state SET clock=?1", [latest])?;
    }
    Ok((outgoing, changed))
}
