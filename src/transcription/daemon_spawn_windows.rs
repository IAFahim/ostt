//! Background Windows processes must not retain the caller's capture pipe handles.
use std::{ffi::OsStr, io, os::windows::ffi::OsStrExt, path::Path, ptr};
use windows_sys::Win32::{
    Foundation::CloseHandle,
    System::Threading::{
        CreateProcessW, CREATE_NEW_PROCESS_GROUP, DETACHED_PROCESS, PROCESS_INFORMATION,
        STARTUPINFOW,
    },
};

fn quoted_arg(arg: &OsStr, output: &mut Vec<u16>) {
    output.push(b'"' as u16);
    let mut slashes = 0;
    for ch in arg.encode_wide() {
        if ch == b'\\' as u16 {
            slashes += 1;
            continue;
        }
        let count = if ch == b'"' as u16 {
            slashes * 2 + 1
        } else {
            slashes
        };
        output.extend(std::iter::repeat_n(b'\\' as u16, count));
        output.push(ch);
        slashes = 0;
    }
    output.extend(std::iter::repeat_n(b'\\' as u16, slashes * 2));
    output.push(b'"' as u16);
}

pub(super) fn spawn_detached(program: &Path, args: &[&str]) -> io::Result<()> {
    let executable: Vec<u16> = program.as_os_str().encode_wide().chain(Some(0)).collect();
    let mut command_line = Vec::new();
    quoted_arg(program.as_os_str(), &mut command_line);
    for arg in args {
        command_line.push(b' ' as u16);
        quoted_arg(OsStr::new(arg), &mut command_line);
    }
    if command_line.contains(&0) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "process argument contains NUL",
        ));
    }
    command_line.push(0);
    let startup = STARTUPINFOW {
        cb: std::mem::size_of::<STARTUPINFOW>() as u32,
        ..Default::default()
    };
    let mut process = PROCESS_INFORMATION::default();
    // std::process::Command inherits unrelated handles on Windows. Even with NUL
    // stdio, that keeps a PowerShell capture open until the daemon exits.
    let result = unsafe {
        CreateProcessW(
            executable.as_ptr(),
            command_line.as_mut_ptr(),
            ptr::null(),
            ptr::null(),
            0,
            DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP,
            ptr::null(),
            ptr::null(),
            &startup,
            &mut process,
        )
    };
    if result == 0 {
        return Err(io::Error::last_os_error());
    }
    unsafe {
        CloseHandle(process.hThread);
        CloseHandle(process.hProcess);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn quoting_preserves_spaces_quotes_and_trailing_backslashes() {
        for (arg, expected) in [
            ("", r#""""#),
            ("two words", r#""two words""#),
            ("a\"b", r#""a\"b""#),
            ("C:\\space dir\\", r#""C:\space dir\\""#),
        ] {
            let mut result = Vec::new();
            quoted_arg(OsStr::new(arg), &mut result);
            assert_eq!(String::from_utf16(&result).unwrap(), expected);
        }
    }
}
