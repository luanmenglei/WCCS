//! 一轮维护轮询使用同一份设备身份与凭据。续权还要检查当前配置是否仍指向该服务器。
use super::journal::Record;
use crate::{AgentResult as Result, connection, identity::Identity};
use reqwest::blocking::Client;
use std::{fs, path::Path};
use wccs_protocol::maintenance::{Command, Poll, Reply};
use wccs_session::ipc;

pub(super) struct Channel<'a> {
    pub(super) client: &'a Client,
    pub(super) path: &'a Path,
    instance: &'a str,
    pub(super) test_mode: bool,
    pub(super) simulation: bool,
    pub(super) identity: Identity,
    pub(super) server: String,
    pub(super) credential: String,
}
impl<'a> Channel<'a> {
    pub(super) fn open(
        client: &'a Client,
        path: &'a Path,
        instance: &'a str,
        test_mode: bool,
        simulation: bool,
    ) -> Result<Self> {
        let identity: Identity = serde_json::from_slice(&fs::read(path)?)?;
        let server = connection::server(path, &identity)?;
        let credential = crate::identity::credential(path, &identity)?;
        Ok(Self {
            client,
            path,
            instance,
            test_mode,
            simulation,
            identity,
            server,
            credential,
        })
    }
    pub(super) fn post_result(&self, record: &Record) -> Result<()> {
        let receipt: serde_json::Value = self
            .client
            .post(format!("{}/api/device/maintenance/result", record.server))
            .bearer_auth(&self.credential)
            .header("X-WCCS-Device-ID", &self.identity.device_id)
            .json(&record.result)
            .send()?
            .error_for_status()?
            .json()?;
        if receipt["accepted"] != true
            || receipt["job_id"] != record.command.job_id
            || receipt["stage"] != record.result.stage
        {
            return Err("maintenance_ack_mismatch".into());
        }
        Ok(())
    }
    pub(super) fn poll(&self) -> Result<Reply> {
        let session = if self.simulation {
            None
        } else {
            ipc::discover(self.test_mode).ok()
        };
        let mut input = Poll {
            schema_version: 1,
            instance_id: self.instance.into(),
            session_key: if self.simulation {
                Some("recovery-simulation".into())
            } else {
                session.map(|s| s.key)
            },
            adapter: if self.test_mode {
                "local_test"
            } else {
                "service"
            }
            .into(),
            capabilities: if self.simulation {
                vec!["recovery-v1".into(), "recovery-v2".into()]
            } else if self.test_mode {
                vec!["identify-v1".into()]
            } else {
                vec![
                    "identify-v1".into(),
                    "preparation-v1".into(),
                    "competition-v1".into(),
                    "recovery-v1".into(),
                    "recovery-v2".into(),
                ]
            },
        };
        let send = |input: &Poll| {
            self.client
                .post(format!("{}/api/device/maintenance/poll", self.server))
                .header("x-wccs-volume-policy", "1")
                .header(
                    "x-wccs-shutdown",
                    if !self.test_mode || self.simulation {
                        "1"
                    } else {
                        "0"
                    },
                )
                .header(
                    "x-wccs-desktop-files",
                    if !self.test_mode { "1" } else { "0" },
                )
                .bearer_auth(&self.credential)
                .header("X-WCCS-Device-ID", &self.identity.device_id)
                .json(input)
                .send()
        };
        let mut response = send(&input)?;
        // Servers predating v2 reject additional capabilities. Keep their v1 operations usable.
        if response.status() == reqwest::StatusCode::BAD_REQUEST {
            input
                .capabilities
                .retain(|capability| capability != "recovery-v2");
            response = send(&input)?;
        }
        let reply: Reply = response.error_for_status()?.json()?;
        if reply.schema_version != 1 {
            return Err("maintenance_protocol_mismatch".into());
        }
        Ok(reply)
    }

    pub(super) fn same_server(&self) -> bool {
        connection::server(self.path, &self.identity)
            .ok()
            .as_deref()
            == Some(self.server.as_str())
    }
    pub(super) fn still_offered(&self, command: &Command) -> Result<bool> {
        Ok(self.same_server() && self.poll()?.command.as_ref() == Some(command))
    }
    pub(super) fn authorize_recovery(&self, command: &Command, command_digest: &str) -> bool {
        let input = Poll {
            schema_version: 1,
            instance_id: self.instance.into(),
            session_key: Some(command.session_key.clone()),
            adapter: if self.test_mode {
                "local_test"
            } else {
                "service"
            }
            .into(),
            capabilities: vec!["recovery-v1".into(), "recovery-v2".into()],
        };
        self.client
            .post(format!("{}/api/device/maintenance/authorize", self.server))
            .header("x-wccs-volume-policy", "1")
            .bearer_auth(&self.credential)
            .header("X-WCCS-Device-ID", &self.identity.device_id)
            .json(&input)
            .send()
            .ok()
            .and_then(|response| response.error_for_status().ok())
            .and_then(|response| response.json::<serde_json::Value>().ok())
            .is_some_and(|reply| {
                reply["schema_version"] == 1 && reply["command_digest"] == command_digest
            })
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    use std::time::Duration;
    #[test]
    fn old_server_capability_rejection_falls_back_to_legacy_poll() {
        use std::io::{Read, Write};
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let server = format!("http://{}", listener.local_addr().unwrap());
        let worker = std::thread::spawn(move || {
            for attempt in 0..2 {
                let (mut stream, _) = listener.accept().unwrap();
                stream
                    .set_read_timeout(Some(Duration::from_secs(5)))
                    .unwrap();
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    let mut byte = [0];
                    stream.read_exact(&mut byte).unwrap();
                    header.push(byte[0]);
                    assert!(header.len() < 8192);
                }
                let header = String::from_utf8(header).unwrap();
                let length: usize = header
                    .lines()
                    .find_map(|line| {
                        let (name, value) = line.split_once(':')?;
                        name.eq_ignore_ascii_case("content-length")
                            .then(|| value.trim().parse().unwrap())
                    })
                    .unwrap();
                let mut bytes = vec![0; length];
                stream.read_exact(&mut bytes).unwrap();
                let input: Poll = serde_json::from_slice(&bytes).unwrap();
                assert_eq!(input.session_key.as_deref(), Some("recovery-simulation"));
                assert_eq!(
                    input.capabilities.iter().any(|cap| cap == "recovery-v2"),
                    attempt == 0
                );
                assert!(input.capabilities.iter().any(|cap| cap == "recovery-v1"));
                let (status, body) = if attempt == 0 {
                    ("400 Bad Request", "{}")
                } else {
                    (
                        "200 OK",
                        "{\"schema_version\":1,\"server_time\":1,\"command\":null}",
                    )
                };
                write!(stream, "HTTP/1.1 {status}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}", body.len()).unwrap();
            }
        });
        let identity: Identity = serde_json::from_value(
            json!({"server":server,"device_id":"synthetic","enrollment_token":"synthetic"}),
        )
        .unwrap();
        let client = Client::builder()
            .no_proxy()
            .timeout(Duration::from_secs(5))
            .build()
            .unwrap();
        assert!(
            Channel {
                client: &client,
                server,
                identity,
                credential: "synthetic".into(),
                instance: "instance",
                path: Path::new("unused"),
                test_mode: true,
                simulation: true
            }
            .poll()
            .unwrap()
            .command
            .is_none()
        );
        worker.join().unwrap();
    }
}
