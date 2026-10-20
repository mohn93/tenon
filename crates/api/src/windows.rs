//! Named pipes for the agent API on Windows: an owner-only pipe server and its client. Every
//! `unsafe` Windows call in this crate lives here.
//!
//! Pipe handles are opened for overlapped I/O, each half waiting on its own event. A handle
//! opened for synchronous I/O serialises every call on it, so a read blocked waiting for the
//! next request would hold up the reply being written from the other thread.

use std::ffi::c_void;
use std::fs::OpenOptions;
use std::io::{self, Read, Write};
use std::os::windows::fs::OpenOptionsExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;
use std::sync::Arc;
use std::time::{Duration, Instant};

use windows_sys::Win32::Foundation::{
    ERROR_ACCESS_DENIED, ERROR_BROKEN_PIPE, ERROR_IO_PENDING, ERROR_PIPE_BUSY,
    ERROR_PIPE_CONNECTED, HANDLE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    FILE_FLAG_FIRST_PIPE_INSTANCE, FILE_FLAG_OVERLAPPED, PIPE_ACCESS_DUPLEX, ReadFile,
    SECURITY_IDENTIFICATION, WriteFile,
};
use windows_sys::Win32::System::IO::{GetOverlappedResult, OVERLAPPED};
use windows_sys::Win32::System::Pipes::{
    ConnectNamedPipe, CreateNamedPipeW, PIPE_READMODE_BYTE, PIPE_REJECT_REMOTE_CLIENTS,
    PIPE_TYPE_BYTE, PIPE_UNLIMITED_INSTANCES, PIPE_WAIT,
};
use windows_sys::Win32::System::Threading::{CreateEventW, GetCurrentProcess, OpenProcessToken};

/// Each direction's pipe buffer, in bytes (a hint to the system).
const BUFFER: u32 = 64 * 1024;

/// How long a client retries while every instance of the pipe is busy (the server creates the
/// next one as soon as a client connects).
const BUSY_WAIT: Duration = Duration::from_secs(5);

/// One end of a named pipe, opened for overlapped I/O.
pub struct Pipe(OwnedHandle);

impl Pipe {
    /// Creates an instance of the pipe `name` (`\\.\pipe\...`) that only the current user can
    /// open and that refuses clients on other machines. With `first`, fails with `AddrInUse`
    /// when the name already exists, so another program cannot take over or share it.
    pub fn create(name: &str, first: bool) -> io::Result<Pipe> {
        let wide = wide(name);
        let descriptor = owner_only_descriptor()?;
        let attributes = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        let mut open_mode = PIPE_ACCESS_DUPLEX | FILE_FLAG_OVERLAPPED;
        if first {
            open_mode |= FILE_FLAG_FIRST_PIPE_INSTANCE;
        }
        let pipe_mode =
            PIPE_TYPE_BYTE | PIPE_READMODE_BYTE | PIPE_WAIT | PIPE_REJECT_REMOTE_CLIENTS;
        // SAFETY: `wide` is NUL-terminated, and it and `attributes` (with the descriptor it
        // points to) outlive the call.
        let handle = unsafe {
            CreateNamedPipeW(
                wide.as_ptr(),
                open_mode,
                pipe_mode,
                PIPE_UNLIMITED_INSTANCES,
                BUFFER,
                BUFFER,
                0,
                &attributes,
            )
        };
        if handle == INVALID_HANDLE_VALUE {
            let e = io::Error::last_os_error();
            // The first instance of a name that exists fails with access denied.
            if first && e.raw_os_error() == Some(ERROR_ACCESS_DENIED as i32) {
                return Err(io::Error::new(
                    io::ErrorKind::AddrInUse,
                    format!("another program is listening on {name}"),
                ));
            }
            return Err(e);
        }
        // SAFETY: `handle` is a valid pipe handle that nothing else owns.
        Ok(Pipe(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }

    /// Waits for a client to connect to this server instance.
    pub fn accept(&self) -> io::Result<()> {
        let event = Event::new()?;
        let mut overlapped = OVERLAPPED {
            hEvent: event.0.as_raw_handle(),
            ..Default::default()
        };
        let handle = self.0.as_raw_handle();
        // SAFETY: `handle` is a pipe server handle opened for overlapped I/O; `overlapped` and
        // its event stay alive until the operation completes (`finish` waits for it).
        if unsafe { ConnectNamedPipe(handle, &mut overlapped) } != 0 {
            return Ok(());
        }
        let e = io::Error::last_os_error();
        match e.raw_os_error() {
            // A client connected between creating the instance and this call.
            Some(code) if code == ERROR_PIPE_CONNECTED as i32 => Ok(()),
            Some(code) if code == ERROR_IO_PENDING as i32 => finish(handle, &overlapped).map(drop),
            _ => Err(e),
        }
    }

    /// The read and write halves, usable from two threads at once.
    pub fn split(self) -> io::Result<(PipeHalf, PipeHalf)> {
        let handle = Arc::new(self.0);
        let reader = PipeHalf {
            pipe: Arc::clone(&handle),
            event: Event::new()?,
        };
        let writer = PipeHalf {
            pipe: handle,
            event: Event::new()?,
        };
        Ok((reader, writer))
    }
}

/// Connects to the pipe `name` as a client. The server may identify the client but not act
/// as it.
pub fn connect(name: &str) -> io::Result<Pipe> {
    let deadline = Instant::now() + BUSY_WAIT;
    loop {
        let opened = OpenOptions::new()
            .read(true)
            .write(true)
            .custom_flags(FILE_FLAG_OVERLAPPED)
            .security_qos_flags(SECURITY_IDENTIFICATION)
            .open(name);
        match opened {
            Ok(file) => return Ok(Pipe(OwnedHandle::from(file))),
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                    && Instant::now() < deadline =>
            {
                std::thread::sleep(Duration::from_millis(10));
            }
            Err(e) => return Err(e),
        }
    }
}

/// One direction of a connected pipe. Reads report the other end closing as end of file.
pub struct PipeHalf {
    pipe: Arc<OwnedHandle>,
    /// Signalled when this half's operation completes.
    event: Event,
}

impl PipeHalf {
    fn overlapped(&self) -> OVERLAPPED {
        OVERLAPPED {
            hEvent: self.event.0.as_raw_handle(),
            ..Default::default()
        }
    }
}

impl Read for PipeHalf {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        let handle = self.pipe.as_raw_handle();
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut overlapped = self.overlapped();
        // SAFETY: `buf` is valid for `len` bytes and, with `overlapped` and its event, stays
        // borrowed until the operation completes (`start` waits for it).
        let started = unsafe {
            ReadFile(
                handle,
                buf.as_mut_ptr(),
                len,
                ptr::null_mut(),
                &mut overlapped,
            )
        };
        match start(handle, started, &overlapped) {
            Err(e) if e.raw_os_error() == Some(ERROR_BROKEN_PIPE as i32) => Ok(0),
            result => result.map(|n| n as usize),
        }
    }
}

impl Write for PipeHalf {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        let handle = self.pipe.as_raw_handle();
        let len = u32::try_from(buf.len()).unwrap_or(u32::MAX);
        let mut overlapped = self.overlapped();
        // SAFETY: `buf` is valid for `len` bytes and, with `overlapped` and its event, stays
        // borrowed until the operation completes (`start` waits for it).
        let started =
            unsafe { WriteFile(handle, buf.as_ptr(), len, ptr::null_mut(), &mut overlapped) };
        start(handle, started, &overlapped).map(|n| n as usize)
    }

    /// A completed write is in the pipe already.
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Completes an overlapped read or write that returned `started`: waits when it is pending.
/// Returns the bytes transferred.
fn start(handle: HANDLE, started: i32, overlapped: &OVERLAPPED) -> io::Result<u32> {
    if started == 0 {
        let e = io::Error::last_os_error();
        if e.raw_os_error() != Some(ERROR_IO_PENDING as i32) {
            return Err(e);
        }
    }
    finish(handle, overlapped)
}

/// Waits for the overlapped operation started on `handle` with `overlapped`.
fn finish(handle: HANDLE, overlapped: &OVERLAPPED) -> io::Result<u32> {
    let mut transferred = 0;
    // SAFETY: `overlapped` belongs to an operation started on `handle`; waiting (the last
    // argument) returns only once it has completed.
    if unsafe { GetOverlappedResult(handle, overlapped, &mut transferred, 1) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(transferred)
}

/// A manual-reset event, not signalled.
struct Event(OwnedHandle);

impl Event {
    fn new() -> io::Result<Event> {
        // SAFETY: no attributes and no name; the returned handle is checked below.
        let handle = unsafe { CreateEventW(ptr::null(), 1, 0, ptr::null()) };
        if handle.is_null() {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `handle` is a fresh event handle that nothing else owns.
        Ok(Event(unsafe { OwnedHandle::from_raw_handle(handle) }))
    }
}

/// Memory the system allocated with `LocalAlloc`, freed on drop.
struct Local(*mut c_void);

impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: the pointer came from a call that allocates with `LocalAlloc` and is freed
        // only here.
        unsafe { LocalFree(self.0) };
    }
}

/// A security descriptor whose protected DACL grants full access to the current user and
/// nothing to anyone else (no inherited entries).
fn owner_only_descriptor() -> io::Result<Local> {
    let sddl = wide(&format!("D:P(A;;GA;;;{})", current_user_sid()?));
    let mut descriptor = ptr::null_mut();
    // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a `LocalAlloc`ed descriptor,
    // owned by `Local` below; the size is not wanted.
    let ok = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(Local(descriptor))
}

/// The current user's SID in string form (`S-1-5-...`), from the process token.
fn current_user_sid() -> io::Result<String> {
    let mut token = ptr::null_mut();
    // SAFETY: the current-process pseudo handle needs no closing; `token` receives a handle,
    // owned below.
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `token` is a fresh handle that nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(token) };
    let mut len = 0u32;
    // SAFETY: an empty buffer asks only for the length needed, written to `len`.
    unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            ptr::null_mut(),
            0,
            &mut len,
        )
    };
    if len == 0 {
        return Err(io::Error::last_os_error());
    }
    // u64s keep the TOKEN_USER (which holds a pointer) aligned.
    let mut buf = vec![0u64; (len as usize).div_ceil(8)];
    // SAFETY: `buf` is at least `len` bytes.
    let ok = unsafe {
        GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            buf.as_mut_ptr().cast(),
            len,
            &mut len,
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: on success `buf` starts with a TOKEN_USER whose SID points into `buf`.
    let sid = unsafe { (*buf.as_ptr().cast::<TOKEN_USER>()).User.Sid };
    let mut text = ptr::null_mut();
    // SAFETY: `sid` is valid while `buf` lives; `text` receives a `LocalAlloc`ed string, owned
    // by `Local` below.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let text = Local(text.cast());
    let start = text.0.cast::<u16>().cast_const();
    let mut len = 0;
    // SAFETY: the string is NUL-terminated, so every unit up to the NUL is readable.
    while unsafe { *start.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: the `len` units before the NUL were just read.
    let units = unsafe { std::slice::from_raw_parts(start, len) };
    String::from_utf16(units).map_err(io::Error::other)
}

/// `s` as a NUL-terminated UTF-16 string.
fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}
