use std::{
    collections::HashSet,
    fs,
    io::{self, Read},
    net::{IpAddr, SocketAddr},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::Duration,
};

use base64::{
    engine::general_purpose::{STANDARD, URL_SAFE_NO_PAD},
    Engine,
};
use reqwest::{
    blocking::{Client, RequestBuilder, Response},
    Certificate, Method, StatusCode,
};
use serde::{Deserialize, Serialize};

use super::{child_dir, hex_valid, read_file, regular_file, EXCHANGE_DIR, MAX_PEERS};

const LEGACY_PREFIX: &str = "yapsync1.";
const INVITE_PREFIX: &str = "yapsync2.";
const REQUEST_TIMEOUT: Duration = Duration::from_secs(12);
const MEDIA_TIMEOUT: Duration = Duration::from_secs(30 * 60);
const TRACK_BATCH: usize = 512;

#[derive(Deserialize)]
pub(crate) struct Invite {
    pub(crate) address: SocketAddr,
    certificate: String,
    #[serde(skip)]
    modern: bool,
}

pub(crate) fn is_lan_address(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => ip.is_private() || ip.is_loopback(),
        IpAddr::V6(ip) => ip.is_unique_local() || ip.is_loopback(),
    }
}

impl Invite {
    pub(crate) fn parse(code: &str) -> io::Result<Self> {
        fn invalid() -> io::Error {
            io::Error::other("Invalid YAP Sync invitation")
        }
        let code = code.trim();
        if code.len() > 8192 {
            return Err(invalid());
        }
        let modern = code.starts_with(INVITE_PREFIX);
        let encoded = code
            .strip_prefix(if modern { INVITE_PREFIX } else { LEGACY_PREFIX })
            .ok_or_else(invalid)?;
        let decoded = URL_SAFE_NO_PAD.decode(encoded).map_err(|_| invalid())?;
        let mut invite: Self = serde_json::from_slice(&decoded).map_err(|_| invalid())?;
        invite.modern = modern;
        if !is_lan_address(invite.address.ip()) || invite.address.port() == 0 {
            return Err(invalid());
        }
        invite.certificate()?;
        Ok(invite)
    }

    pub(crate) fn modern(&self) -> bool {
        self.modern
    }

    pub(crate) fn certificate(&self) -> io::Result<Certificate> {
        let pem = STANDARD
            .decode(&self.certificate)
            .map_err(io::Error::other)?;
        Certificate::from_pem(&pem).map_err(io::Error::other)
    }

    pub(crate) fn url(&self, route: &str) -> String {
        format!("https://{}/v1{route}", self.address)
    }
}

#[derive(Serialize)]
pub(crate) struct DeviceAuth {
    pub(crate) id: String,
    pub(crate) key: String,
    pub(crate) name: String,
}

pub(super) enum Access {
    Allowed,
    Pending,
    Blocked,
    Upgrade,
}

pub(super) struct Directory {
    pub(super) root: PathBuf,
    pub(super) tracks: PathBuf,
    pub(super) devices: PathBuf,
    pub(super) repairs: PathBuf,
}

pub(super) enum Exchange {
    Directory(Directory),
    Server(Server),
}

impl Exchange {
    pub(super) fn directory(root: PathBuf) -> io::Result<Self> {
        let shared = child_dir(&root, EXCHANGE_DIR)?;
        Ok(Self::Directory(Directory {
            root,
            tracks: child_dir(&shared, "tracks")?,
            devices: child_dir(&shared, "devices")?,
            repairs: child_dir(&shared, "repairs")?,
        }))
    }

    pub(super) fn list(&self) -> io::Result<Vec<String>> {
        let mut ids = match self {
            Self::Server(server) => {
                let body = read_response(
                    server
                        .request(Method::GET, "/devices")
                        .send()
                        .map_err(io::Error::other)?,
                    16 * 1024,
                )?
                .ok_or_else(|| io::Error::other("Server did not provide a device list"))?;
                serde_json::from_slice::<Vec<String>>(&body).map_err(io::Error::other)?
            }
            Self::Directory(directory) => {
                let mut ids = Vec::new();
                for entry in fs::read_dir(&directory.devices)? {
                    let entry = entry?;
                    let name = entry.file_name();
                    if let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".json")) {
                        if hex_valid(id, 32) {
                            ids.push(id.to_owned());
                            if ids.len() > MAX_PEERS {
                                return Err(io::Error::other("Too many devices"));
                            }
                        }
                    }
                }
                ids
            }
        };
        if ids.len() > MAX_PEERS || ids.iter().any(|id| !hex_valid(id, 32)) {
            return Err(io::Error::other("Invalid device list"));
        }
        ids.sort_unstable();
        ids.dedup();
        Ok(ids)
    }

    pub(super) fn manifest(&self, id: &str, limit: u64) -> io::Result<Option<Vec<u8>>> {
        match self {
            Self::Server(server) => read_response(
                server
                    .request(Method::GET, &format!("/devices/{id}"))
                    .send()
                    .map_err(io::Error::other)?,
                limit,
            ),
            Self::Directory(directory) => {
                match read_file(&directory.devices.join(format!("{id}.json")), limit) {
                    Ok(data) => Ok(Some(data)),
                    Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(None),
                    Err(error) => Err(error),
                }
            }
        }
    }

    pub(super) fn publish(
        &self,
        id: &str,
        data: Vec<u8>,
        previous: Option<&[u8]>,
    ) -> io::Result<usize> {
        if previous == Some(data.as_slice()) {
            return Ok(0);
        }
        match self {
            Self::Server(server) => {
                let bytes = data.len();
                server
                    .request(Method::PUT, &format!("/devices/{id}"))
                    .body(data)
                    .send()
                    .and_then(Response::error_for_status)
                    .map_err(io::Error::other)?;
                Ok(bytes)
            }
            Self::Directory(directory) => {
                use std::io::Write;
                let mut file = tempfile::NamedTempFile::new_in(&directory.devices)?;
                file.write_all(&data)?;
                file.as_file().sync_all()?;
                file.persist(directory.devices.join(format!("{id}.json")))?;
                Ok(data.len())
            }
        }
    }

    pub(super) fn signal_repair(&self, digest: &str) {
        match self {
            Self::Server(server) => {
                let _ = server
                    .request(Method::POST, &format!("/tracks/{digest}/repair"))
                    .send();
            }
            Self::Directory(directory) => {
                let _ = fs::OpenOptions::new()
                    .write(true)
                    .create_new(true)
                    .open(directory.repairs.join(digest));
            }
        }
    }
}

fn read_response(reply: Response, limit: u64) -> io::Result<Option<Vec<u8>>> {
    if reply.status() == StatusCode::NOT_FOUND {
        return Ok(None);
    }
    let reply = reply.error_for_status().map_err(io::Error::other)?;
    if reply.content_length().is_some_and(|size| size > limit) {
        return Err(io::Error::other("Sync response is too large"));
    }
    let mut bytes = Vec::new();
    reply.take(limit + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > limit {
        return Err(io::Error::other("Sync response is too large"));
    }
    Ok(Some(bytes))
}

pub(super) struct Server {
    client: Client,
    uploads: reqwest::Client,
    invite: Invite,
    device: DeviceAuth,
}

impl Server {
    pub(super) fn new(invite: Invite, device: DeviceAuth) -> io::Result<Self> {
        let client = Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .timeout(REQUEST_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(invite.certificate()?)
            .build()
            .map_err(io::Error::other)?;
        let uploads = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(4))
            .read_timeout(REQUEST_TIMEOUT)
            .timeout(MEDIA_TIMEOUT)
            .redirect(reqwest::redirect::Policy::none())
            .https_only(true)
            .no_proxy()
            .tls_built_in_root_certs(false)
            .add_root_certificate(invite.certificate()?)
            .build()
            .map_err(io::Error::other)?;
        Ok(Self {
            client,
            uploads,
            invite,
            device,
        })
    }

    fn request(&self, method: Method, path: &str) -> RequestBuilder {
        self.client
            .request(method, self.invite.url(path))
            .header("X-Yap-Device", &self.device.id)
            .bearer_auth(&self.device.key)
    }

    pub(super) fn check(&self) -> io::Result<Access> {
        #[derive(Deserialize)]
        struct Protocol {
            version: u32,
            #[serde(default)]
            access: u32,
        }
        let mut reply = self
            .request(Method::GET, "")
            .send()
            .map_err(io::Error::other)?;
        if reply.status() == StatusCode::UNAUTHORIZED {
            #[derive(Deserialize)]
            struct Pairing {
                status: String,
            }
            let body = serde_json::to_vec(&self.device).map_err(io::Error::other)?;
            let pairing = self
                .client
                .post(self.invite.url("/pair"))
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .map_err(io::Error::other)?;
            if matches!(
                pairing.status(),
                StatusCode::UNAUTHORIZED | StatusCode::NOT_FOUND | StatusCode::METHOD_NOT_ALLOWED
            ) {
                return Ok(Access::Upgrade);
            }
            let response = read_response(pairing, 1024)?
                .ok_or_else(|| io::Error::other("Missing pairing status"))?;
            let paired: Pairing = serde_json::from_slice(&response).map_err(io::Error::other)?;
            match paired.status.as_str() {
                "pending" => return Ok(Access::Pending),
                "blocked" => return Ok(Access::Blocked),
                "allowed" => {
                    reply = self
                        .request(Method::GET, "")
                        .send()
                        .map_err(io::Error::other)?
                }
                _ => return Err(io::Error::other("Invalid pairing status")),
            }
        }
        let body =
            read_response(reply, 1024)?.ok_or_else(|| io::Error::other("Missing sync protocol"))?;
        let protocol: Protocol = serde_json::from_slice(&body).map_err(io::Error::other)?;
        if protocol.version != super::VERSION || protocol.access != 2 {
            return Ok(Access::Upgrade);
        }
        Ok(Access::Allowed)
    }

    pub(super) fn present<'a>(
        &self,
        mut tracks: impl Iterator<Item = (&'a str, u64)>,
        cancelled: &AtomicBool,
    ) -> io::Result<HashSet<String>> {
        #[derive(Serialize)]
        struct Track<'a> {
            digest: &'a str,
            bytes: u64,
        }
        let mut present = HashSet::new();
        loop {
            if cancelled.load(Ordering::Relaxed) {
                return Err(io::Error::other("Sync cancelled"));
            }
            let batch: Vec<_> = tracks
                .by_ref()
                .take(TRACK_BATCH)
                .map(|(digest, bytes)| Track { digest, bytes })
                .collect();
            if batch.is_empty() {
                return Ok(present);
            }
            let body = serde_json::to_vec(&batch).map_err(io::Error::other)?;
            let reply = self
                .request(Method::POST, "/tracks/present")
                .header(reqwest::header::CONTENT_TYPE, "application/json")
                .body(body)
                .send()
                .map_err(io::Error::other)?;
            let bytes = read_response(reply, 64 * 1024)?
                .ok_or_else(|| io::Error::other("Server did not provide a track list"))?;
            let digests: Vec<String> = serde_json::from_slice(&bytes).map_err(io::Error::other)?;
            if digests.len() > batch.len() || digests.iter().any(|digest| !hex_valid(digest, 64)) {
                return Err(io::Error::other("Invalid server track list"));
            }
            present.extend(digests);
        }
    }

    pub(super) fn send(
        &self,
        digest: &str,
        path: &Path,
        size: u64,
        check_presence: bool,
        cancelled: &AtomicBool,
    ) -> io::Result<u64> {
        let endpoint = format!("/tracks/{digest}");
        if check_presence {
            let head = self
                .request(Method::HEAD, &endpoint)
                .send()
                .map_err(io::Error::other)?;
            match head.status() {
                StatusCode::OK
                    if head
                        .headers()
                        .get(reqwest::header::CONTENT_LENGTH)
                        .and_then(|value| value.to_str().ok())
                        .and_then(|value| value.parse::<u64>().ok())
                        == Some(size)
                        && !head.headers().contains_key("X-Yap-Repair") =>
                {
                    return Ok(0)
                }
                StatusCode::OK | StatusCode::NOT_FOUND => {}
                _ => {
                    head.error_for_status().map_err(io::Error::other)?;
                }
            }
        }
        let source = regular_file(path, size)?;
        if source.metadata()?.len() != size {
            return Err(io::Error::other("Local track size changed"));
        }
        let url = self.invite.url(&endpoint);
        tauri::async_runtime::block_on(async {
            let reader = tokio_util::io::ReaderStream::new(tokio::fs::File::from_std(source));
            let upload = self
                .uploads
                .put(url)
                .header("X-Yap-Device", &self.device.id)
                .bearer_auth(&self.device.key)
                .header(reqwest::header::CONTENT_LENGTH, size)
                .body(reqwest::Body::wrap_stream(reader))
                .send();
            tokio::pin!(upload);
            loop {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(io::Error::other("Sync cancelled"));
                }
                tokio::select! {
                    response = &mut upload => {
                        response.and_then(reqwest::Response::error_for_status).map_err(io::Error::other)?;
                        return Ok(size);
                    }
                    _ = tokio::time::sleep(Duration::from_millis(100)) => {}
                }
            }
        })
    }

    pub(super) fn receive(&self, digest: &str, size: u64) -> io::Result<Response> {
        let response = self
            .request(Method::GET, &format!("/tracks/{digest}"))
            .send()
            .and_then(Response::error_for_status)
            .map_err(io::Error::other)?;
        if response.content_length() != Some(size) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "Incorrect server track size",
            ));
        }
        Ok(response)
    }
}
