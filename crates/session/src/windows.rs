//! Owner-only files and folders on Windows: created with a protected DACL that grants full
//! control to the current user and nothing to anyone else. Every `unsafe` Windows call in this
//! crate lives here.

use std::ffi::c_void;
use std::fs::File;
use std::io;
use std::os::windows::ffi::OsStrExt;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::path::Path;
use std::ptr;

use windows_sys::Win32::Foundation::{
    GENERIC_READ, GENERIC_WRITE, INVALID_HANDLE_VALUE, LocalFree,
};
use windows_sys::Win32::Security::Authorization::{
    ConvertSecurityDescriptorToStringSecurityDescriptorW, ConvertSidToStringSidW,
    ConvertStringSecurityDescriptorToSecurityDescriptorW, GetNamedSecurityInfoW, SDDL_REVISION_1,
    SE_FILE_OBJECT,
};
use windows_sys::Win32::Security::{
    DACL_SECURITY_INFORMATION, GetTokenInformation, OWNER_SECURITY_INFORMATION,
    SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
};
use windows_sys::Win32::Storage::FileSystem::{
    CREATE_NEW, CreateDirectoryW, CreateFileW, FILE_ATTRIBUTE_NORMAL, FILE_SHARE_DELETE,
    FILE_SHARE_READ, FILE_SHARE_WRITE,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// Creates the file `path`, failing with `AlreadyExists` when anything is there (so it never
/// goes through a link), open for writing, readable and writable by the current user only.
pub fn create_private_file(path: &Path) -> io::Result<File> {
    let descriptor = Descriptor::owner_only("")?;
    let wide = wide(path)?;
    let attributes = descriptor.attributes();
    // SAFETY: `wide` is NUL-terminated; it and `attributes` (with the descriptor it points
    // to) outlive the call; no template file.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            GENERIC_READ | GENERIC_WRITE,
            FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE,
            &attributes,
            CREATE_NEW,
            FILE_ATTRIBUTE_NORMAL,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is a fresh file handle that nothing else owns.
    Ok(File::from(unsafe { OwnedHandle::from_raw_handle(handle) }))
}

/// Creates the folder `path` (its parent must exist) for the current user only; files and
/// folders created in it inherit that.
pub fn create_private_dir(path: &Path) -> io::Result<()> {
    let descriptor = Descriptor::owner_only("OICI")?;
    let wide = wide(path)?;
    let attributes = descriptor.attributes();
    // SAFETY: `wide` is NUL-terminated; it and `attributes` (with the descriptor it points
    // to) outlive the call.
    if unsafe { CreateDirectoryW(wide.as_ptr(), &attributes) } == 0 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

/// The owner and DACL of the file or folder at `path`, in SDDL (`O:<sid>D:P(A;...)`).
pub fn security_text(path: &Path) -> io::Result<String> {
    let wide = wide(path)?;
    let info = OWNER_SECURITY_INFORMATION | DACL_SECURITY_INFORMATION;
    let mut descriptor = ptr::null_mut();
    // SAFETY: `wide` is NUL-terminated; the owner and DACL pointers are not wanted (they
    // would point into the descriptor); `descriptor` receives a `LocalAlloc`ed descriptor,
    // owned by `Local` below.
    let status = unsafe {
        GetNamedSecurityInfoW(
            wide.as_ptr(),
            SE_FILE_OBJECT,
            info,
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            ptr::null_mut(),
            &mut descriptor,
        )
    };
    if status != 0 {
        return Err(io::Error::from_raw_os_error(status as i32));
    }
    let descriptor = Local(descriptor);
    let mut text = ptr::null_mut();
    // SAFETY: `descriptor` is valid; `text` receives a `LocalAlloc`ed string, owned by
    // `Local` below; the length is not wanted.
    let ok = unsafe {
        ConvertSecurityDescriptorToStringSecurityDescriptorW(
            descriptor.0,
            SDDL_REVISION_1,
            info,
            &mut text,
            ptr::null_mut(),
        )
    };
    if ok == 0 {
        return Err(io::Error::last_os_error());
    }
    let text = Local(text.cast());
    // SAFETY: the system wrote a NUL-terminated string there.
    unsafe { local_text(&text) }
}

/// The current user's SID in string form (`S-1-5-...`).
pub fn current_user() -> io::Result<String> {
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
    // SAFETY: `sid` is a valid SID while `buf` lives; `text` receives a `LocalAlloc`ed
    // string, owned by `Local` below.
    if unsafe { ConvertSidToStringSidW(sid, &mut text) } == 0 {
        return Err(io::Error::last_os_error());
    }
    let text = Local(text.cast());
    // SAFETY: the system wrote a NUL-terminated string there.
    unsafe { local_text(&text) }
}

/// A security descriptor built from SDDL.
struct Descriptor(Local);

impl Descriptor {
    /// Owned by the current user, whose protected DACL grants that user full control and no
    /// one else anything; `inherit` is the entry's inheritance flags (`OICI` for a folder).
    fn owner_only(inherit: &str) -> io::Result<Descriptor> {
        let user = current_user()?;
        let sddl = format!("O:{user}D:P(A;{inherit};FA;;;{user})");
        let sddl: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut descriptor = ptr::null_mut();
        // SAFETY: `sddl` is NUL-terminated; `descriptor` receives a `LocalAlloc`ed
        // descriptor, owned by `Local` below; the size is not wanted.
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
        Ok(Descriptor(Local(descriptor)))
    }

    /// Attributes pointing at this descriptor; valid while it lives.
    fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0.0,
            bInheritHandle: 0,
        }
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

/// The NUL-terminated UTF-16 string in `local`.
///
/// # Safety
///
/// `local` must hold a NUL-terminated UTF-16 string.
unsafe fn local_text(local: &Local) -> io::Result<String> {
    let start = local.0.cast::<u16>().cast_const();
    let mut len = 0;
    // SAFETY: the string is NUL-terminated (the caller's promise), so every unit up to the
    // NUL is readable.
    while unsafe { *start.add(len) } != 0 {
        len += 1;
    }
    // SAFETY: the `len` units before the NUL were just read.
    let units = unsafe { std::slice::from_raw_parts(start, len) };
    String::from_utf16(units).map_err(io::Error::other)
}

/// `path` as a NUL-terminated UTF-16 string; a NUL inside it is an error.
fn wide(path: &Path) -> io::Result<Vec<u16>> {
    let units: Vec<u16> = path.as_os_str().encode_wide().collect();
    if units.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a path cannot contain a NUL",
        ));
    }
    Ok(units.into_iter().chain(std::iter::once(0)).collect())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(name: &str) -> std::path::PathBuf {
        let path = std::env::temp_dir().join(format!("session-win-{}-{name}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        let _ = std::fs::remove_file(&path);
        path
    }

    #[test]
    fn private_files_and_folders_are_the_owners_only() {
        let user = current_user().unwrap();
        let dir = scratch("dir");
        create_private_dir(&dir).unwrap();
        let text = security_text(&dir).unwrap();
        assert_eq!(
            owner_and_entries(&text),
            (user.clone(), true, vec![format!("A;OICI;FA;;;{user}")]),
            "{text}"
        );
        // A file made inside inherits it.
        std::fs::write(dir.join("inherits"), b"x").unwrap();
        let text = security_text(&dir.join("inherits")).unwrap();
        let (_, _, entries) = owner_and_entries(&text);
        assert_eq!(entries.len(), 1, "{text}");
        assert!(entries[0].ends_with(&format!(";FA;;;{user}")), "{text}");
        let file = dir.join("private");
        drop(create_private_file(&file).unwrap());
        let text = security_text(&file).unwrap();
        assert_eq!(
            owner_and_entries(&text),
            (user.clone(), true, vec![format!("A;;FA;;;{user}")]),
            "{text}"
        );
        let e = create_private_file(&file).unwrap_err();
        assert_eq!(e.kind(), io::ErrorKind::AlreadyExists);
        assert!(create_private_dir(&dir).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }

    /// The owner, whether the DACL is protected, and its entries, from SDDL.
    fn owner_and_entries(sddl: &str) -> (String, bool, Vec<String>) {
        let rest = sddl.strip_prefix("O:").expect("an owner");
        let (owner, dacl) = rest.split_once("D:").expect("a DACL");
        let (flags, entries) = dacl.split_once('(').unwrap_or((dacl, ""));
        let entries = format!("({entries}")
            .split(['(', ')'])
            .filter(|e| !e.is_empty())
            .map(String::from)
            .collect();
        (owner.to_string(), flags.contains('P'), entries)
    }
}
