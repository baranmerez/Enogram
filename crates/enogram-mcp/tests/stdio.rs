use serde_json::{json, Value};
use std::fs;
use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, Command, Stdio};
use std::sync::mpsc::{self, Receiver};
use std::thread;
use std::time::{Duration, Instant};

struct Client {
    child: Child,
    input: Option<ChildStdin>,
    output: Receiver<String>,
}
impl Client {
    fn send(&mut self, request: Value) {
        writeln!(self.input.as_mut().unwrap(), "{request}").unwrap();
        self.input.as_mut().unwrap().flush().unwrap();
    }
    fn read(&self) -> Value {
        let line = self
            .output
            .recv_timeout(Duration::from_secs(15))
            .expect("bounded stdio response deadline");
        serde_json::from_str(&line).expect("stdout is protocol-only JSON")
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        self.input.take();
        if self.child.try_wait().ok().flatten().is_none() {
            let _ = self.child.kill();
        }
        let _ = self.child.wait();
    }
}

#[test]
fn compiled_stdio_probe_initialize_tools_parse_recovery_and_eof() {
    let root = std::env::temp_dir().join(format!("enogram-mcp-stdio-{}", std::process::id()));
    fs::create_dir(&root).unwrap();
    let init = Command::new("git")
        .args(["init", "-b", "main"])
        .arg(&root)
        .output()
        .unwrap();
    assert!(init.status.success());
    let mut child = Command::new(env!("CARGO_BIN_EXE_enogram-mcp"))
        .arg("--repo")
        .arg(&root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    let (sender, output) = mpsc::channel();
    let stdout = child.stdout.take().unwrap();
    let reader = thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            if sender.send(line.unwrap()).is_err() {
                break;
            }
        }
    });
    let input = child.stdin.take();
    let mut client = Client {
        child,
        input,
        output,
    };
    client.send(json!({"jsonrpc":"2.0","id":1,"method":"server/discover"}));
    assert_eq!(client.read()["error"]["code"], -32601);
    client.send(json!({"jsonrpc":"2.0","id":2,"method":"initialize","params":{
        "protocolVersion":"2025-11-25","capabilities":{},"clientInfo":{"name":"binary-fixture","version":"1"}}}));
    assert_eq!(client.read()["result"]["protocolVersion"], "2025-11-25");
    client.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));
    client.send(json!({"jsonrpc":"2.0","id":3,"method":"tools/list"}));
    assert_eq!(
        client.read()["result"]["tools"].as_array().unwrap().len(),
        4
    );
    client.send(json!({"jsonrpc":"2.0","id":4,"method":"tools/call","params":{"name":"enogram_list_repositories","arguments":{}}}));
    let listed = client.read();
    assert_eq!(
        listed["result"]["structuredContent"]["repositories"][0]["repoId"],
        "repo-1"
    );
    writeln!(client.input.as_mut().unwrap(), "invalid-json").unwrap();
    client.input.as_mut().unwrap().flush().unwrap();
    assert_eq!(client.read()["error"]["code"], -32700);
    client.send(json!({"jsonrpc":"2.0","id":5,"method":"ping"}));
    assert_eq!(client.read()["result"], json!({}));
    client.input.take();
    let start = Instant::now();
    loop {
        if let Some(status) = client.child.try_wait().unwrap() {
            assert!(status.success());
            break;
        }
        assert!(
            start.elapsed() < Duration::from_secs(5),
            "EOF must exit without a shutdown request"
        );
        thread::sleep(Duration::from_millis(10));
    }
    reader.join().unwrap();
    assert!(
        client.output.try_recv().is_err(),
        "No unsolicited stdout messages"
    );
    drop(client);
    assert!(root.starts_with(std::env::temp_dir()));
    assert!(root
        .file_name()
        .unwrap()
        .to_string_lossy()
        .starts_with("enogram-mcp-stdio-"));
    fs::remove_dir_all(root).unwrap();
    let invalid = Command::new(env!("CARGO_BIN_EXE_enogram-mcp"))
        .arg("--allow-write")
        .output()
        .unwrap();
    assert_eq!(invalid.status.code(), Some(2));
    assert!(invalid.stdout.is_empty());
}
