#![cfg(windows)]
#![forbid(unsafe_code)]

use serde_json::{Value, json};
use std::{
    fs,
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    process::Command,
    thread,
    time::Duration,
};
use wccs_protocol::{now, token};

fn request(stream: &mut TcpStream) -> (String, Value) {
    stream
        .set_read_timeout(Some(Duration::from_secs(15)))
        .unwrap();
    let mut bytes = Vec::new();
    let mut buffer = [0u8; 1024];
    let header_end = loop {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
        if let Some(index) = bytes.windows(4).position(|slice| slice == b"\r\n\r\n") {
            break index + 4;
        }
    };
    let header = String::from_utf8(bytes[..header_end].to_vec()).unwrap();
    let length: usize = header
        .lines()
        .find_map(|line| {
            line.to_lowercase()
                .strip_prefix("content-length: ")
                .map(str::to_string)
        })
        .unwrap()
        .trim()
        .parse()
        .unwrap();
    while bytes.len() < header_end + length {
        let count = stream.read(&mut buffer).unwrap();
        assert!(count > 0);
        bytes.extend_from_slice(&buffer[..count]);
    }
    (
        header.lines().next().unwrap().to_string(),
        serde_json::from_slice(&bytes[header_end..header_end + length]).unwrap(),
    )
}
fn reply(stream: &mut TcpStream, value: &Value) {
    let body = serde_json::to_vec(value).unwrap();
    write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",body.len()).unwrap();
    stream.write_all(&body).unwrap();
}

#[test]
fn credential_rotation_recovers_before_and_after_commit() {
    for committed in [false, true] {
        let directory = std::env::temp_dir().join(format!("wccs-rotation-{}", token()));
        fs::create_dir(&directory).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let identity = directory.join("device.json");
        let old = token();
        fs::write(&identity,serde_json::to_vec(&json!({"server":format!("http://{}",listener.local_addr().unwrap()),"device_id":"rotation","enrollment_token":token(),"device_token":old})).unwrap()).unwrap();
        let worker = thread::spawn(move || {
            let heartbeat = json!({"protocol_version":1,"server_time":now(),"lifecycle":"enabled","rotation_id":"rotate-test","job":null});
            let (mut stream, _) = listener.accept().unwrap();
            assert!(request(&mut stream).0.contains("heartbeat"));
            reply(&mut stream, &heartbeat);
            drop(stream);
            let (mut stream, _) = listener.accept().unwrap();
            let (path, first) = request(&mut stream);
            assert!(path.contains("rotate"));
            drop(stream);
            // The subprocess exits with its durable candidate after losing the response.
            let (mut stream, _) = listener.accept().unwrap();
            assert!(request(&mut stream).0.contains("heartbeat"));
            if committed {
                write!(stream,"HTTP/1.1 401 Unauthorized\r\nContent-Length: 2\r\nConnection: close\r\n\r\n{{}}").unwrap();
                drop(stream);
                let (mut stream, _) = listener.accept().unwrap();
                assert!(request(&mut stream).0.contains("heartbeat"));
                reply(
                    &mut stream,
                    &json!({"protocol_version":1,"server_time":now(),"lifecycle":"enabled","rotation_id":null,"job":null}),
                );
            } else {
                reply(&mut stream, &heartbeat);
                drop(stream);
                let (mut stream, _) = listener.accept().unwrap();
                let (path, second) = request(&mut stream);
                assert!(path.contains("rotate"));
                assert_eq!(first, second);
                reply(&mut stream, &json!({"ok":true}));
            }
            first["new_token"].as_str().unwrap().to_string()
        });
        let run = || {
            Command::new(env!("CARGO_BIN_EXE_wccs-agent"))
                .arg("--identity")
                .arg(&identity)
                .arg("--once")
                .output()
                .unwrap()
        };
        assert!(!run().status.success());
        assert!(identity.with_extension("rotation.json").exists());
        assert_eq!(
            fs::read_to_string(identity.with_extension("credential")).unwrap(),
            old
        );
        assert!(run().status.success());
        assert_eq!(
            fs::read_to_string(identity.with_extension("credential")).unwrap(),
            worker.join().unwrap()
        );
        assert!(!identity.with_extension("rotation.json").exists());
        fs::remove_dir_all(directory).unwrap();
    }
}

#[test]
fn lost_result_response_retries_identical_persisted_payload() {
    let directory = std::env::temp_dir().join(format!("wccs-agent-{}", token()));
    fs::create_dir(&directory).unwrap();
    let script = directory.join("合成 脚本.ps1");
    fs::write(&script, "# read-only test\n").unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let server = format!("http://{}", listener.local_addr().unwrap());
    let identity = directory.join("device.json");
    fs::write(&identity,serde_json::to_vec(&json!({"server":server,"device_id":"synthetic","enrollment_token":token(),"device_token":token()})).unwrap()).unwrap();
    let heartbeat = json!({"protocol_version":1,"server_time":now(),"job":{"job_id":"job-1","expires_at":now()+60,"configuration":{"schema_version":1,"config_version":"1","tools":[{"id":"script","name":"合成脚本","kind":"script","path":script},{"id":"unset","name":"未配置","kind":"script","path":null}]}}});
    let worker = thread::spawn(move || {
        let (mut stream, _) = listener.accept().unwrap();
        assert!(request(&mut stream).0.contains("heartbeat"));
        reply(&mut stream, &heartbeat);
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        let (path, first) = request(&mut stream);
        assert!(path.contains("result"));
        assert_eq!(first["tools"][0]["check"]["status"], "available");
        assert_eq!(first["tools"][1]["check"]["reason"], "not_configured");
        // Simulate a committed server write whose HTTP response is lost.
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        assert!(request(&mut stream).0.contains("heartbeat"));
        reply(&mut stream, &heartbeat);
        drop(stream);
        let (mut stream, _) = listener.accept().unwrap();
        let (_, second) = request(&mut stream);
        assert_eq!(first, second);
        reply(&mut stream, &json!({"accepted":true,"stale":false}));
    });
    let run = || {
        Command::new(env!("CARGO_BIN_EXE_wccs-agent"))
            .arg("--identity")
            .arg(&identity)
            .arg("--once")
            .output()
            .unwrap()
    };
    assert!(!run().status.success());
    assert!(identity.with_extension("pending.json").exists());
    assert!(run().status.success());
    assert!(!identity.with_extension("pending.json").exists());
    assert_eq!(
        fs::read_to_string(identity.with_extension("completed")).unwrap(),
        "job-1"
    );
    worker.join().unwrap();
    // The directory is uniquely created by this test under the OS temporary path.
    fs::remove_dir_all(directory).unwrap();
}
