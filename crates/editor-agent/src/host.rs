use crate::*;
use std::{
    fs,
    io::{BufRead, BufReader, Read, Write},
    net::{SocketAddr, TcpListener, TcpStream},
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU8, Ordering},
        mpsc::{self, Receiver, SyncSender},
    },
    thread,
    time::Duration,
};
use uuid::Uuid;

#[derive(Debug, thiserror::Error)]
pub enum AgentError {
    #[error("Agent transport: {0}")]
    Io(#[from] std::io::Error),
    #[error("Agent protocol: {0}")]
    Json(#[from] serde_json::Error),
    #[error("{0}")]
    Invalid(String),
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDescriptor {
    pub protocol_version: u32,
    pub address: SocketAddr,
    pub token: String,
    pub instance_id: Uuid,
}
#[derive(Serialize, Deserialize)]
struct Envelope {
    token: String,
    request: AgentRequest,
}

const QUEUED: u8 = 0;
const EXECUTING: u8 = 1;
const CANCELLED: u8 = 2;
const COMPLETED: u8 = 3;
pub struct AgentCall {
    pub request: AgentRequest,
    response: SyncSender<AgentResponse>,
    state: Arc<AtomicU8>,
}
impl AgentCall {
    pub fn is_live(&self) -> bool {
        self.state.load(Ordering::Acquire) == QUEUED
    }
    pub fn reply(self, response: AgentResponse) {
        self.state.store(COMPLETED, Ordering::Release);
        let _ = self.response.try_send(response);
    }
    /// Takes the call for an answer made on the owning thread, only if the
    /// requester is still waiting. `None` means the call expired and nothing
    /// may be started for it.
    pub fn claim(self) -> Option<Claimed> {
        self.state
            .compare_exchange(QUEUED, EXECUTING, Ordering::AcqRel, Ordering::Acquire)
            .ok()?;
        Some(Claimed {
            request: self.request,
            response: self.response,
            state: self.state,
        })
    }
    /// Apply on the desktop's owning thread only if the requester is still waiting.
    pub fn execute(self, editor: &mut Editor) {
        if let Some(call) = self.claim() {
            let response = handle(editor, call.request.clone());
            call.reply(response);
        }
    }
}

/// A call its requester still waits on, answered exactly once by `reply`.
/// Dropping it unanswered leaves the requester with `indeterminate`.
pub struct Claimed {
    pub request: AgentRequest,
    response: SyncSender<AgentResponse>,
    state: Arc<AtomicU8>,
}
impl Claimed {
    pub fn reply(self, response: AgentResponse) {
        self.state.store(COMPLETED, Ordering::Release);
        let _ = self.response.try_send(response);
    }
}

pub struct AgentHost {
    descriptor: SessionDescriptor,
    requests: Receiver<AgentCall>,
    stopped: Arc<AtomicBool>,
    published: Vec<PathBuf>,
    document: Arc<Mutex<Option<(u64, DocumentInfo)>>>,
}
impl AgentHost {
    pub fn new(wake: impl Fn() + Send + Sync + 'static) -> Result<Self, AgentError> {
        let listener = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))?;
        let descriptor = SessionDescriptor {
            protocol_version: PROTOCOL_VERSION,
            address: listener.local_addr()?,
            token: Uuid::new_v4().to_string(),
            instance_id: Uuid::new_v4(),
        };
        let (sender, requests) = mpsc::sync_channel(16);
        let stopped = Arc::new(AtomicBool::new(false));
        let document = Arc::new(Mutex::new(None::<(u64, DocumentInfo)>));
        let shown = document.clone();
        let worker_stopped = stopped.clone();
        let token = descriptor.token.clone();
        thread::Builder::new().name("neptune-video-agent".into()).spawn(move||{
            while !worker_stopped.load(Ordering::Acquire){
                let Ok((mut stream,address))=listener.accept() else{break;};
                if worker_stopped.load(Ordering::Acquire){break;}
                if !address.ip().is_loopback(){continue;}
                let _=stream.set_read_timeout(Some(Duration::from_secs(2)));let _=stream.set_write_timeout(Some(Duration::from_secs(2)));
                let result=read_envelope(&stream);
                let response=match result {
                    // The document as the owner last published it is answered
                    // here, so watching a session never wakes its window.
                    Ok(envelope) if same_token(&envelope.token,&token) && matches!(envelope.request.operation,AgentOperation::Document) && published(&shown).is_some()=>{
                        let (revision,document)=published(&shown).unwrap_or_else(||unreachable!());
                        AgentResponse{request_id:envelope.request.request_id,revision,result:AgentResult::Document{document:Box::new(document)}}
                    },
                    Ok(envelope) if same_token(&envelope.token,&token)=>{
                        let request_id=envelope.request.request_id.clone();let (reply,receiver)=mpsc::sync_channel(1);let state=Arc::new(AtomicU8::new(QUEUED));
                        let call=AgentCall{request:envelope.request,response:reply,state:state.clone()};
                        if sender.try_send(call).is_err(){AgentResponse::error(request_id,0,"busy","Agent request queue is full")}
                        else {
                            wake();match receiver.recv_timeout(Duration::from_secs(5)){
                                Ok(response)=>response,
                                Err(_)=>{
                                    if state.compare_exchange(QUEUED,CANCELLED,Ordering::AcqRel,Ordering::Acquire).is_ok(){AgentResponse::error(request_id,0,"timeout","Request expired before the editor accepted it; no edit was applied")}
                                    else{AgentResponse::error(request_id,0,"indeterminate","Request was accepted but its acknowledgement exceeded the deadline; inspect the session before retrying")}
                                }
                            }
                        }
                    },
                    Ok(envelope)=>AgentResponse::error(envelope.request.request_id,0,"unauthorized","Session token does not match"),
                    Err(error)=>AgentResponse::error(String::new(),0,"protocol",error.to_string()),
                };
                let _=serde_json::to_writer(&mut stream,&response);let _=stream.write_all(b"\n");
            }
        })?;
        Ok(Self {
            descriptor,
            requests,
            stopped,
            published: Vec::new(),
            document,
        })
    }
    /// Publishes the document as the owner sees it at `revision`; `document`
    /// requests are then answered from it without waking the owner. Cheap
    /// when nothing changed.
    pub fn publish_document(&self, revision: u64, document: DocumentInfo) {
        let Ok(mut shown) = self.document.lock() else {
            return;
        };
        if shown
            .as_ref()
            .is_none_or(|(at, known)| *at != revision || *known != document)
        {
            *shown = Some((revision, document));
        }
    }
    pub fn descriptor(&self) -> &SessionDescriptor {
        &self.descriptor
    }
    pub fn poll(&self) -> Option<AgentCall> {
        self.requests.try_recv().ok()
    }
    /// Publish in a task/session-specific location. The token file is private.
    pub fn publish(&mut self, path: &Path) -> Result<(), AgentError> {
        let parent = path
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or_else(|| Path::new("."));
        fs::create_dir_all(parent)?;
        let mut file = tempfile::NamedTempFile::new_in(parent)?;
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            file.as_file()
                .set_permissions(fs::Permissions::from_mode(0o600))?;
        }
        serde_json::to_writer(file.as_file_mut(), &self.descriptor)?;
        file.as_file_mut().write_all(b"\n")?;
        file.as_file().sync_all()?;
        file.persist(path).map_err(|e| e.error)?;
        self.published.push(path.into());
        Ok(())
    }
}
impl Drop for AgentHost {
    fn drop(&mut self) {
        self.stopped.store(true, Ordering::Release);
        let _ = TcpStream::connect_timeout(&self.descriptor.address, Duration::from_millis(100));
        for path in &self.published {
            if fs::read(path)
                .ok()
                .and_then(|b| serde_json::from_slice::<SessionDescriptor>(&b).ok())
                .is_some_and(|d| d.instance_id == self.descriptor.instance_id)
            {
                let _ = fs::remove_file(path);
            }
        }
    }
}
fn published(shown: &Mutex<Option<(u64, DocumentInfo)>>) -> Option<(u64, DocumentInfo)> {
    shown.lock().ok().and_then(|shown| shown.clone())
}
fn read_envelope(stream: &TcpStream) -> Result<Envelope, AgentError> {
    let mut reader = BufReader::new(stream.take(MAX_REQUEST_BYTES + 1));
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line)?;
    if line.len() as u64 > MAX_REQUEST_BYTES || !line.ends_with(b"\n") {
        return Err(AgentError::Invalid(
            "Agent requests must be newline-delimited JSON under 16 MiB".into(),
        ));
    }
    Ok(serde_json::from_slice(&line)?)
}
pub fn connect(
    descriptor: &SessionDescriptor,
    request: AgentRequest,
) -> Result<AgentResponse, AgentError> {
    connect_within(
        descriptor,
        request,
        Duration::from_secs(2),
        Duration::from_secs(8),
    )
}

/// `connect` with its own connect and read deadlines: a client that looks
/// for sessions asks briefly; one waiting on an edit waits longer than the
/// host's own five seconds.
pub fn connect_within(
    descriptor: &SessionDescriptor,
    request: AgentRequest,
    connect: Duration,
    read: Duration,
) -> Result<AgentResponse, AgentError> {
    if descriptor.protocol_version != PROTOCOL_VERSION || !descriptor.address.ip().is_loopback() {
        return Err(AgentError::Invalid(
            "Unsupported or non-local agent session".into(),
        ));
    }
    let mut stream = TcpStream::connect_timeout(&descriptor.address, connect)?;
    stream.set_read_timeout(Some(read))?;
    stream.set_write_timeout(Some(Duration::from_secs(2)))?;
    let bytes = serde_json::to_vec(&Envelope {
        token: descriptor.token.clone(),
        request,
    })?;
    if bytes.len() as u64 > MAX_REQUEST_BYTES {
        return Err(AgentError::Invalid(format!(
            "Agent request exceeds {} MiB",
            MAX_REQUEST_BYTES / (1024 * 1024)
        )));
    }
    stream.write_all(&bytes)?;
    stream.write_all(b"\n")?;
    let mut reader = BufReader::new(stream.take(256 * 1024 * 1024));
    let mut line = Vec::new();
    reader.read_until(b'\n', &mut line)?;
    Ok(serde_json::from_slice(&line)?)
}

/// Compares two tokens in time that depends only on their lengths, so a
/// local process cannot learn a token a byte at a time.
pub fn same_token(given: &str, expected: &str) -> bool {
    let (given, expected) = (given.as_bytes(), expected.as_bytes());
    let mut differ = given.len() ^ expected.len();
    for (index, byte) in expected.iter().enumerate() {
        differ |= usize::from(byte ^ given.get(index).copied().unwrap_or(!byte));
    }
    differ == 0
}
