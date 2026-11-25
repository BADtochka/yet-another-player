use rusqlite::{params, Connection, Transaction};
use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

use super::{digest_valid, Manifest, MAX_CLOCK};
use crate::library::{edits, queries, LibraryResult, Tags};

const FIELDS: [&str; 8] = [
    "name",
    "artists",
    "album",
    "albumArtist",
    "releaseDate",
    "genres",
    "trackNumber",
    "discNumber",
];

#[derive(Deserialize, Serialize)]
pub(super) struct Edit {
    digest: String,
    field: String,
    clock: i64,
    value: Value,
}

fn valid_value(field: &str, value: &Value) -> bool {
    let text = |value: &Value| {
        value.as_str().is_some_and(|text| {
            !text.trim().is_empty() && text.len() <= 512 && !text.chars().any(char::is_control)
        })
    };
    match field {
        "name" => text(value),
        "artists" | "genres" => value
            .as_array()
            .is_some_and(|values| values.len() <= 32 && values.iter().all(text)),
        "album" | "albumArtist" => value.is_null() || text(value),
        "releaseDate" => {
            value.is_null()
                || value.as_str().is_some_and(|date| {
                    matches!(date.len(), 4 | 7 | 10)
                        && date.bytes().enumerate().all(|(index, ch)| {
                            if index == 4 || index == 7 {
                                ch == b'-'
                            } else {
                                ch.is_ascii_digit()
                            }
                        })
                })
        }
        "discNumber" | "trackNumber" => {
            value.is_null()
                || value
                    .as_u64()
                    .is_some_and(|number| u32::try_from(number).is_ok())
        }
        _ => false,
    }
}

impl Edit {
    pub(super) fn digest(&self) -> &str {
        &self.digest
    }

    pub(super) fn valid(&self) -> bool {
        digest_valid(&self.digest)
            && (1..MAX_CLOCK).contains(&self.clock)
            && valid_value(&self.field, &self.value)
    }
}

pub(in crate::library) fn queue(
    db: &Connection,
    track: i64,
    fields: &Map<String, Value>,
) -> LibraryResult<()> {
    let clock: i64 = db.query_row(
        "UPDATE sync_state SET clock=clock+1 WHERE clock+1<?1 RETURNING clock",
        [MAX_CLOCK],
        |row| row.get(0),
    )?;
    let mut statement = db.prepare_cached(
        "INSERT INTO sync_pending_metadata VALUES (?1,?2,?3,?4)
         ON CONFLICT(track_id,field) DO UPDATE SET clock=excluded.clock,value=excluded.value",
    )?;
    for field in FIELDS {
        if let Some(value) = fields.get(field) {
            statement.execute(params![track, field, clock, serde_json::to_string(value)?])?;
        }
    }
    Ok(())
}

pub(in crate::library) fn extend(
    db: &Connection,
    path: &str,
    fields: &mut Map<String, Value>,
) -> LibraryResult<()> {
    let mut statement = db.prepare_cached(
        "SELECT remote.field,remote.value FROM track t
         JOIN sync_identity identity ON identity.track_id=t.id AND identity.stamp=t.stamp AND identity.size=t.size
         JOIN sync_metadata remote ON remote.digest=identity.digest
         LEFT JOIN sync_pending_metadata pending ON pending.track_id=t.id AND pending.field=remote.field
         WHERE t.path=?1 AND (pending.clock IS NULL OR remote.clock>=pending.clock)",
    )?;
    let mut rows = statement.query([path])?;
    while let Some(row) = rows.next()? {
        fields.insert(
            row.get(0)?,
            serde_json::from_str(&row.get::<_, String>(1)?)?,
        );
    }
    Ok(())
}

pub(super) fn merge(
    db: &Transaction<'_>,
    device: &str,
    peers: &[Manifest],
) -> LibraryResult<(Vec<Edit>, bool)> {
    db.execute(
        "INSERT INTO sync_metadata
         SELECT identity.digest, pending.field, pending.clock, ?1, pending.value
         FROM sync_pending_metadata pending JOIN track t ON t.id=pending.track_id
         JOIN sync_identity identity ON identity.track_id=t.id AND identity.stamp=t.stamp AND identity.size=t.size
         ORDER BY pending.track_id
         ON CONFLICT(digest,field) DO UPDATE SET clock=excluded.clock,device=excluded.device,value=excluded.value
         WHERE (excluded.clock,excluded.device)>(clock,device)",
        [device],
    )?;
    db.execute(
        "DELETE FROM sync_pending_metadata AS pending WHERE EXISTS (
           SELECT 1 FROM sync_identity identity JOIN track t ON t.id=identity.track_id
           WHERE identity.track_id=pending.track_id AND identity.stamp=t.stamp AND identity.size=t.size
         )", [],
    )?;
    let mut latest = 0;
    {
        let mut statement = db.prepare_cached(
            "INSERT INTO sync_metadata VALUES (?1,?2,?3,?4,?5)
             ON CONFLICT(digest,field) DO UPDATE SET clock=excluded.clock,device=excluded.device,value=excluded.value
             WHERE (excluded.clock,excluded.device)>(clock,device)",
        )?;
        for peer in peers {
            for edit in &peer.metadata_edits {
                statement.execute(params![
                    edit.digest,
                    edit.field,
                    edit.clock,
                    peer.device,
                    serde_json::to_string(&edit.value)?
                ])?;
                latest = latest.max(edit.clock);
            }
        }
    }
    db.execute("UPDATE sync_state SET clock=?1 WHERE clock<?1", [latest])?;

    let mut changed = false;
    let mut statement = db.prepare_cached(
        "SELECT t.id,t.path,t.metadata FROM (SELECT DISTINCT digest FROM sync_metadata) m
         JOIN sync_identity identity ON identity.digest=m.digest
         JOIN track t ON t.id=identity.track_id AND identity.stamp=t.stamp AND identity.size=t.size",
    )?;
    let tracks = statement
        .query_map([], |row| {
            Ok((
                row.get::<_, i64>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .collect::<rusqlite::Result<Vec<_>>>()?;
    for (id, path, raw) in tracks {
        let mut tags: Tags = serde_json::from_str(&raw)?;
        edits::apply(db, &path, &mut tags)?;
        let next = serde_json::to_string(&tags)?;
        if next != raw {
            db.execute(
                "UPDATE track SET metadata=?2 WHERE id=?1",
                params![id, next],
            )?;
            queries::index(db, id, &path, &tags)?;
            changed = true;
        }
    }
    let mut statement = db.prepare_cached(
        "SELECT digest,field,clock,value FROM sync_metadata WHERE device=?1 ORDER BY digest,field",
    )?;
    let mut rows = statement.query([device])?;
    let mut outgoing = Vec::new();
    while let Some(row) = rows.next()? {
        let edit = Edit {
            digest: row.get(0)?,
            field: row.get(1)?,
            clock: row.get(2)?,
            value: serde_json::from_str(&row.get::<_, String>(3)?)?,
        };
        if edit.valid() {
            outgoing.push(edit);
        }
    }
    Ok((outgoing, changed))
}
