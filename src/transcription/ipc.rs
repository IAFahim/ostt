//! Platform transport for the local daemon; framing remains shared.
#[cfg(unix)]
pub(super) use tokio::net::{UnixListener as Listener, UnixStream as Client, UnixStream as Server};

#[cfg(unix)]
pub(super) fn bind() -> std::io::Result<Listener> {
    Listener::bind(super::local_models::daemon_socket_path())
}

#[cfg(unix)]
pub(super) async fn connect() -> std::io::Result<Client> {
    Client::connect(super::local_models::daemon_socket_path()).await
}

#[cfg(windows)]
pub(super) use windows::*;

#[cfg(windows)]
mod windows {
    use crate::windows_ipc::{user_sid, UserSecurity};
    use std::{io, time::Duration};
    use tokio::net::windows::named_pipe::{
        ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
    };
    use tokio::time::{sleep_until, Instant};
    use windows_sys::Win32::Foundation::ERROR_PIPE_BUSY;

    pub(in crate::transcription) type Client = NamedPipeClient;
    pub(in crate::transcription) type Server = NamedPipeServer;

    pub(in crate::transcription) struct Listener {
        name: String,
        pending: NamedPipeServer,
    }

    fn pipe_name() -> io::Result<String> {
        Ok(format!(r"\\.\pipe\ostt-daemon-{}", user_sid()?))
    }

    fn create(name: &str, first: bool) -> io::Result<Server> {
        let security = UserSecurity::new()?;
        let mut attrs = security.attributes();
        // User-only access, local clients only, and exclusive first instance.
        unsafe {
            ServerOptions::new()
                .reject_remote_clients(true)
                .first_pipe_instance(first)
                .create_with_security_attributes_raw(
                    name,
                    (&mut attrs as *mut windows_sys::Win32::Security::SECURITY_ATTRIBUTES).cast(),
                )
        }
    }

    pub(in crate::transcription) fn bind() -> io::Result<Listener> {
        let name = pipe_name()?;
        let pending = create(&name, true)?;
        Ok(Listener { name, pending })
    }

    impl Listener {
        pub(in crate::transcription) async fn accept(&mut self) -> io::Result<(Server, ())> {
            self.pending.connect().await?;
            // Keep an instance listening while the accepted request is processed.
            let next = create(&self.name, false)?;
            Ok((std::mem::replace(&mut self.pending, next), ()))
        }
    }

    pub(in crate::transcription) async fn connect() -> io::Result<Client> {
        connect_to(&pipe_name()?).await
    }

    async fn connect_to(name: &str) -> io::Result<Client> {
        let deadline = Instant::now() + Duration::from_secs(2);
        loop {
            match ClientOptions::new().open(name) {
                Ok(client) => return Ok(client),
                Err(error)
                    if error.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                        && Instant::now() < deadline =>
                {
                    // Another caller may have connected before accept replaces the instance.
                    sleep_until((Instant::now() + Duration::from_millis(25)).min(deadline)).await;
                }
                Err(error) => return Err(error),
            }
        }
    }
    #[cfg(test)]
    mod tests {
        use super::*;
        use std::{
            os::windows::io::AsRawHandle,
            ptr,
            time::{Duration, SystemTime, UNIX_EPOCH},
        };
        use tokio::io::{AsyncReadExt, AsyncWriteExt};
        use windows_sys::Win32::{
            Foundation::LocalFree,
            Security::{
                Authorization::{ConvertStringSidToSidW, GetSecurityInfo, SE_KERNEL_OBJECT},
                EqualSid, GetAce, ACCESS_ALLOWED_ACE, DACL_SECURITY_INFORMATION,
            },
        };

        fn unique_pipe_name() -> String {
            format!(
                "{}-test-{}-{}",
                pipe_name().unwrap(),
                std::process::id(),
                SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap()
                    .as_nanos()
            )
        }

        #[tokio::test]
        async fn connection_waits_for_busy_instance_but_missing_pipe_fails_promptly() {
            let name = unique_pipe_name();
            let pending = create(&name, true).unwrap();
            let mut listener = Listener {
                name: name.clone(),
                pending,
            };
            let _first_client = connect_to(&name).await.unwrap();
            listener.pending.connect().await.unwrap();

            let waiting = connect_to(&name);
            tokio::pin!(waiting);
            assert!(
                tokio::time::timeout(Duration::from_millis(100), waiting.as_mut())
                    .await
                    .is_err(),
                "a transient busy instance must wait instead of appearing unavailable"
            );
            let (_first_server, ()) = listener.accept().await.unwrap();
            let mut queued_client = tokio::time::timeout(Duration::from_secs(1), waiting.as_mut())
                .await
                .expect("connection must resume when accept creates a listening instance")
                .unwrap();
            let (mut second_server, ()) = listener.accept().await.unwrap();
            queued_client
                .write_all(b"queued transcription")
                .await
                .unwrap();
            let mut bytes = [0; 20];
            second_server.read_exact(&mut bytes).await.unwrap();
            assert_eq!(&bytes, b"queued transcription");

            let missing = unique_pipe_name();
            let error = tokio::time::timeout(Duration::from_millis(100), connect_to(&missing))
                .await
                .expect("missing endpoints must fail without the busy-instance retry delay")
                .unwrap_err();
            assert_eq!(error.kind(), io::ErrorKind::NotFound);
        }

        #[tokio::test]
        async fn daemon_pipe_is_exclusive_user_restricted_and_exchanges_frames() {
            let name = unique_pipe_name();
            let pending = create(&name, true).unwrap();
            assert!(
                create(&name, true).is_err(),
                "a second daemon must not bind the same endpoint"
            );
            // Inspect the live pipe's ACL: broad default Windows ACLs are insufficient.
            unsafe {
                let mut acl = ptr::null_mut();
                let mut descriptor = ptr::null_mut();
                assert_eq!(
                    GetSecurityInfo(
                        pending.as_raw_handle(),
                        SE_KERNEL_OBJECT,
                        DACL_SECURITY_INFORMATION,
                        ptr::null_mut(),
                        ptr::null_mut(),
                        &mut acl,
                        ptr::null_mut(),
                        &mut descriptor
                    ),
                    0
                );
                assert!(!acl.is_null());
                assert_eq!(
                    (*acl).AceCount,
                    1,
                    "only the current user may access daemon audio/text"
                );
                let mut ace = ptr::null_mut();
                assert_ne!(GetAce(acl, 0, &mut ace), 0);
                let allowed = &*ace.cast::<ACCESS_ALLOWED_ACE>();
                assert_eq!(allowed.Header.AceType, 0); // ACCESS_ALLOWED_ACE_TYPE
                let mut expected_sid = ptr::null_mut();
                assert_ne!(
                    ConvertStringSidToSidW(
                        crate::windows_ipc::wide(&user_sid().unwrap()).as_ptr(),
                        &mut expected_sid
                    ),
                    0
                );
                let matches = EqualSid(
                    (&allowed.SidStart as *const u32).cast_mut().cast(),
                    expected_sid,
                );
                LocalFree(expected_sid);
                LocalFree(descriptor);
                assert_ne!(
                    matches, 0,
                    "pipe access must belong to the calling Windows user"
                );
            }
            let mut listener = Listener {
                name: name.clone(),
                pending,
            };
            tokio::time::timeout(Duration::from_secs(3), async {
                // Multiple requests prove the listener retains an instance across connections.
                for text in ["first transcription", "second transcription"] {
                    let mut client = connect_to(&name).await.unwrap();
                    let (mut server, ()) = listener.accept().await.unwrap();
                    let payload = text.as_bytes();
                    client
                        .write_all(&(payload.len() as u32).to_le_bytes())
                        .await
                        .unwrap();
                    client.write_all(payload).await.unwrap();
                    let len = server.read_u32_le().await.unwrap();
                    let mut received = vec![0; len as usize];
                    server.read_exact(&mut received).await.unwrap();
                    assert_eq!(received, payload);
                    server.write_all(&len.to_le_bytes()).await.unwrap();
                    server.write_all(&received).await.unwrap();
                    assert_eq!(client.read_u32_le().await.unwrap(), len);
                    let mut echoed = vec![0; len as usize];
                    client.read_exact(&mut echoed).await.unwrap();
                    assert_eq!(echoed, payload);
                }
            })
            .await
            .expect("local daemon IPC exchange timed out");
            drop(listener);
            assert!(
                ClientOptions::new().open(&name).is_err(),
                "pipe must disappear when the daemon exits"
            );
        }
    }
}
