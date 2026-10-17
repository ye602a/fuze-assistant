//! Async WinHTTP: only owner thread starts operations; CloseHandle cancels pending IO.
//! Callback context/buffer remain owned until HANDLE_CLOSING, including failed handshakes.
use crate::{events::Fragments, transport::Header, Credentials, Error, Result};
use std::{
    cell::UnsafeCell,
    ffi::c_void,
    ptr::{null, null_mut},
    sync::{
        mpsc::{self, Receiver, SyncSender},
        Arc,
    },
    thread::Thread,
    time::Duration,
};
use windows_sys::Win32::Networking::WinHttp::*;
struct Handle(*mut c_void);
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            WinHttpCloseHandle(self.0);
        }
    }
}
#[derive(Clone, Copy)]
struct Event {
    status: u32,
    size: u32,
    kind: i32,
}
struct State {
    send: SyncSender<Event>,
    wake: Thread,
    header: Header,
    buffer: UnsafeCell<[u8; 4096]>,
    overflow: std::sync::atomic::AtomicBool,
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        time::Instant,
    };
    use windows_sys::Win32::Security::Cryptography::*;
    #[test]
    fn loopback_upgrade_fragments_and_pending_receive_cancellation() {
        for rejected in [false, true] {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let server = std::thread::spawn(move || {
                let deadline = Instant::now() + Duration::from_secs(5);
                let mut stream = loop {
                    if let Ok((stream, _)) = listener.accept() {
                        break stream;
                    }
                    assert!(Instant::now() < deadline, "fixture-accept-timeout");
                    std::thread::sleep(Duration::from_millis(10));
                };
                stream.set_nonblocking(false).unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(3)))
                    .unwrap();
                let mut request = Vec::new();
                while !request.ends_with(b"\r\n\r\n") {
                    let mut b = [0];
                    stream.read_exact(&mut b).unwrap();
                    request.push(b[0]);
                    assert!(request.len() < 8192);
                }
                if rejected {
                    stream.write_all(b"HTTP/1.1 401 Unauthorized\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").unwrap();
                    return;
                }
                let text = std::str::from_utf8(&request).unwrap();
                let key = text
                    .lines()
                    .find_map(|line| {
                        line.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("Sec-WebSocket-Key"))
                            .map(|(_, v)| v.trim())
                    })
                    .unwrap();
                let source = format!("{key}258EAFA5-E914-47DA-95CA-C5AB0DC85B11");
                let mut hash = [0u8; 20];
                let mut encoded = [0u8; 64];
                let mut length = encoded.len() as u32;
                unsafe {
                    assert_eq!(
                        BCryptHash(
                            BCRYPT_SHA1_ALG_HANDLE,
                            null(),
                            0,
                            source.as_ptr(),
                            source.len() as u32,
                            hash.as_mut_ptr(),
                            20
                        ),
                        0
                    );
                    assert_ne!(
                        CryptBinaryToStringA(
                            hash.as_ptr(),
                            20,
                            CRYPT_STRING_BASE64 | CRYPT_STRING_NOCRLF,
                            encoded.as_mut_ptr(),
                            &mut length
                        ),
                        0
                    );
                }
                let accept = std::str::from_utf8(&encoded[..length as usize])
                    .unwrap()
                    .trim_end_matches('\0');
                stream.write_all(format!("HTTP/1.1 101 Switching Protocols\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Accept: {accept}\r\n\r\n").as_bytes()).unwrap();
                let mut subscription = [0u8; 128];
                assert!(stream.read(&mut subscription).unwrap() > 0);
                let event=br#"[8,"OnJsonApiEvent",{"uri":"/riotclient/region-locale","eventType":"Update","data":{"locale":"zh_CN"}}]"#;
                stream.write_all(&[1, 20]).unwrap();
                stream.write_all(&event[..20]).unwrap();
                stream.write_all(&[128, (event.len() - 20) as u8]).unwrap();
                stream.write_all(&event[20..]).unwrap();
                // The client cancels a pending async receive; peer must observe closure.
                let mut end = [0u8; 64];
                let _ = stream.read(&mut end);
            });
            let c = Credentials::new(7, port, "fixture-only").unwrap();
            let result = Socket::connect_inner(&c, false);
            if rejected {
                assert!(matches!(result, Err(Error::Unauthorized)));
            } else {
                let mut socket = result.unwrap();
                let weak = Arc::downgrade(&socket.state);
                let deadline = Instant::now() + Duration::from_secs(3);
                while !socket.poll().unwrap() {
                    assert!(Instant::now() < deadline, "fixture-event-timeout");
                    std::thread::park_timeout(Duration::from_millis(20));
                }
                assert!(socket.event_observed);
                drop(socket);
                while weak.upgrade().is_some() {
                    assert!(Instant::now() < deadline, "callback-context-not-released");
                    std::thread::sleep(Duration::from_millis(10));
                }
            }
            server.join().unwrap();
        }
    }
}
// WinHTTP alone writes the buffer while a single receive is pending. The owner reads only
// after READ_COMPLETE arrives through the channel, before it starts the next receive.
unsafe impl Sync for State {}
unsafe extern "system" fn callback(
    _: *mut c_void,
    context: usize,
    status: u32,
    info: *mut c_void,
    length: u32,
) {
    if context == 0 {
        return;
    }
    if status == WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING {
        drop(Arc::from_raw(context as *const State));
        return;
    }
    let state = &*(context as *const State);
    let mut event = Event {
        status,
        size: 0,
        kind: 0,
    };
    match status {
        WINHTTP_CALLBACK_STATUS_READ_COMPLETE => {
            if info.is_null() || length < std::mem::size_of::<WINHTTP_WEB_SOCKET_STATUS>() as u32 {
                event.status = WINHTTP_CALLBACK_STATUS_REQUEST_ERROR;
            } else {
                let value = &*(info as *const WINHTTP_WEB_SOCKET_STATUS);
                event.size = value.dwBytesTransferred;
                event.kind = value.eBufferType;
            }
        }
        WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE
        | WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE
        | WINHTTP_CALLBACK_STATUS_WRITE_COMPLETE
        | WINHTTP_CALLBACK_STATUS_REQUEST_ERROR => {}
        _ => return,
    }
    if state.send.try_send(event).is_err() {
        state
            .overflow
            .store(true, std::sync::atomic::Ordering::Release);
    }
    state.wake.unpark();
}
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(Some(0)).collect()
}
fn handle(raw: *mut c_void) -> Result<Handle> {
    if raw.is_null() {
        Err(Error::Transport)
    } else {
        Ok(Handle(raw))
    }
}
fn check(ok: i32) -> Result<()> {
    if ok == 0 {
        Err(Error::Transport)
    } else {
        Ok(())
    }
}
fn option(handle: &Handle, name: u32, value: u32) -> Result<()> {
    check(unsafe { WinHttpSetOption(handle.0, name, (&value as *const u32).cast(), 4) })
}
fn await_status(receive: &Receiver<Event>, expected: u32) -> Result<()> {
    let event = receive
        .recv_timeout(Duration::from_secs(2))
        .map_err(|_| Error::Transport)?;
    if event.status != expected {
        return Err(Error::Transport);
    }
    Ok(())
}
pub struct Socket {
    socket: Handle,
    _request: Handle,
    _connection: Handle,
    _session: Handle,
    state: Arc<State>,
    receive: Receiver<Event>,
    fragments: Fragments,
    pending: bool,
    pub event_observed: bool,
}
impl Socket {
    pub(crate) fn connect(credentials: &Credentials) -> Result<Self> {
        Self::connect_inner(credentials, true)
    }
    fn connect_inner(credentials: &Credentials, secure: bool) -> Result<Self> {
        unsafe {
            let session = handle(WinHttpOpen(
                wide("Fuze-Rust-events").as_ptr(),
                WINHTTP_ACCESS_TYPE_NO_PROXY,
                null(),
                null(),
                WINHTTP_FLAG_ASYNC,
            ))?;
            check(WinHttpSetTimeouts(session.0, 500, 500, 500, 500))?;
            let connection = handle(WinHttpConnect(
                session.0,
                wide("127.0.0.1").as_ptr(),
                credentials.port,
                0,
            ))?;
            let request = handle(WinHttpOpenRequest(
                connection.0,
                wide("GET").as_ptr(),
                wide("/").as_ptr(),
                null(),
                null(),
                null(),
                if secure { WINHTTP_FLAG_SECURE } else { 0 },
            ))?;
            option(
                &request,
                WINHTTP_OPTION_REDIRECT_POLICY,
                WINHTTP_OPTION_REDIRECT_POLICY_NEVER,
            )?;
            option(
                &request,
                WINHTTP_OPTION_DISABLE_FEATURE,
                WINHTTP_DISABLE_COOKIES | WINHTTP_DISABLE_AUTHENTICATION,
            )?;
            if secure {
                option(
                    &request,
                    WINHTTP_OPTION_SECURITY_FLAGS,
                    SECURITY_FLAG_IGNORE_UNKNOWN_CA | SECURITY_FLAG_IGNORE_CERT_CN_INVALID,
                )?;
            }
            check(WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_UPGRADE_TO_WEB_SOCKET,
                null(),
                0,
            ))?;
            let (send, receive) = mpsc::sync_channel(8);
            let state = Arc::new(State {
                send,
                wake: std::thread::current(),
                header: Header::new(credentials),
                buffer: UnsafeCell::new([0; 4096]),
                overflow: std::sync::atomic::AtomicBool::new(false),
            });
            let mask = WINHTTP_CALLBACK_STATUS_HANDLE_CLOSING
                | WINHTTP_CALLBACK_STATUS_READ_COMPLETE
                | WINHTTP_CALLBACK_STATUS_WRITE_COMPLETE
                | WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE
                | WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE
                | WINHTTP_CALLBACK_STATUS_REQUEST_ERROR;
            let previous = WinHttpSetStatusCallback(request.0, Some(callback), mask, 0);
            if previous.is_some_and(|f| f as usize == usize::MAX) {
                return Err(Error::Transport);
            }
            let context = Arc::into_raw(state.clone()) as usize;
            if WinHttpSetOption(
                request.0,
                WINHTTP_OPTION_CONTEXT_VALUE,
                (&context as *const usize).cast(),
                std::mem::size_of::<usize>() as u32,
            ) == 0
            {
                drop(Arc::from_raw(context as *const State));
                return Err(Error::Transport);
            }
            check(WinHttpSendRequest(
                request.0,
                state.header.0.as_ptr(),
                state.header.0.len() as u32,
                null(),
                0,
                0,
                context,
            ))?;
            await_status(&receive, WINHTTP_CALLBACK_STATUS_SENDREQUEST_COMPLETE)?;
            check(WinHttpReceiveResponse(request.0, null_mut()))?;
            await_status(&receive, WINHTTP_CALLBACK_STATUS_HEADERS_AVAILABLE)?;
            let mut status = 0u32;
            let mut size = 4;
            check(WinHttpQueryHeaders(
                request.0,
                WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
                null(),
                (&mut status as *mut u32).cast(),
                &mut size,
                null_mut(),
            ))?;
            if status != 101 {
                return Err(if status == 401 || status == 403 {
                    Error::Unauthorized
                } else {
                    Error::HttpStatus
                });
            }
            let context = Arc::into_raw(state.clone()) as usize;
            let raw = WinHttpWebSocketCompleteUpgrade(request.0, context);
            if raw.is_null() {
                drop(Arc::from_raw(context as *const State));
                return Err(Error::Transport);
            }
            let socket = Handle(raw);
            // Match the existing LCU WAMP subscription. Only relevant events leave the parser.
            static SUBSCRIPTION: &[u8] = br#"[5,"OnJsonApiEvent"]"#;
            if WinHttpWebSocketSend(
                socket.0,
                WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE,
                SUBSCRIPTION.as_ptr().cast(),
                SUBSCRIPTION.len() as u32,
            ) != 0
            {
                return Err(Error::Transport);
            }
            await_status(&receive, WINHTTP_CALLBACK_STATUS_WRITE_COMPLETE)?;
            let mut this = Self {
                socket,
                _request: request,
                _connection: connection,
                _session: session,
                state,
                receive,
                fragments: Fragments::default(),
                pending: false,
                event_observed: false,
            };
            this.start_receive()?;
            Ok(this)
        }
    }
    fn start_receive(&mut self) -> Result<()> {
        if self.pending {
            return Ok(());
        }
        let result = unsafe {
            WinHttpWebSocketReceive(
                self.socket.0,
                (*self.state.buffer.get()).as_mut_ptr().cast(),
                4096,
                null_mut(),
                null_mut(),
            )
        };
        if result != 0 {
            return Err(Error::Transport);
        }
        self.pending = true;
        Ok(())
    }
    /// At most 8 completions per call; coalesced boolean, never an unbounded event queue.
    pub fn poll(&mut self) -> Result<bool> {
        if self
            .state
            .overflow
            .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(Error::ResponseLimit);
        }
        let mut dirty = false;
        for _ in 0..8 {
            let Ok(event) = self.receive.try_recv() else {
                break;
            };
            if event.status != WINHTTP_CALLBACK_STATUS_READ_COMPLETE || event.size > 4096 {
                return Err(Error::Transport);
            }
            if !matches!(
                event.kind,
                WINHTTP_WEB_SOCKET_UTF8_FRAGMENT_BUFFER_TYPE
                    | WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE
            ) {
                return Err(Error::Transport);
            }
            self.pending = false;
            let bytes = unsafe { &(&*self.state.buffer.get())[..event.size as usize] };
            let relevant = self.fragments.push(
                bytes,
                event.kind == WINHTTP_WEB_SOCKET_UTF8_MESSAGE_BUFFER_TYPE,
            )?;
            dirty |= relevant;
            self.event_observed |= relevant;
            self.start_receive()?;
        }
        Ok(dirty)
    }
}
