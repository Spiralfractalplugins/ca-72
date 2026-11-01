//! The built bridge, `ca72-mcp`, as an agent's MCP client starts it: MCP on its standard input
//! and output, passing the calls to a CA-72's socket (here a socket of the plug-in's own
//! server, its requests answered by the test as the editor would answer them). Its process
//! killed mid-request leaves the instance serving, the request dropped (decisions.md R-MCP).
#![cfg(unix)]

use std::io::{BufRead, BufReader, Write};
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use ca72_plugin::agent::server::Server;
use serde_json::{Value, json};

struct Bridge {
    child: Child,
    input: Option<ChildStdin>,
    output: BufReader<ChildStdout>,
    id: u64,
}

impl Bridge {
    fn start(dir: &std::path::Path) -> Bridge {
        let mut child = Command::new(env!("CARGO_BIN_EXE_ca72-mcp"))
            .env("CA72_AGENT_DIR", dir)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .spawn()
            .expect("ca72-mcp");
        let input = child.stdin.take();
        let output = BufReader::new(child.stdout.take().expect("stdout"));
        Bridge {
            child,
            input,
            output,
            id: 0,
        }
    }

    fn send(&mut self, method: &str, params: Value) -> u64 {
        self.id += 1;
        let msg = json!({"jsonrpc": "2.0", "id": self.id, "method": method, "params": params});
        writeln!(self.input.as_mut().expect("open"), "{msg}").expect("write");
        self.id
    }

    fn ask(&mut self, method: &str, params: Value) -> Value {
        let id = self.send(method, params);
        let mut line = String::new();
        self.output.read_line(&mut line).expect("read");
        let v: Value = serde_json::from_str(&line).expect("json");
        assert_eq!(v["id"], json!(id));
        v
    }

    fn tool(&mut self, name: &str, args: Value) -> Value {
        self.ask("tools/call", json!({"name": name, "arguments": args}))["result"].clone()
    }
}

/// The test as the editor: requests answered while `go` is set. Holding its lock
/// through the drain makes pausing an acknowledgement, not a racing flag update.
fn editor(
    s: &Server,
    go: Arc<Mutex<bool>>,
    stop: Arc<AtomicBool>,
    done: Arc<AtomicUsize>,
) -> std::thread::JoinHandle<()> {
    let inbox = Arc::clone(s.inbox());
    std::thread::spawn(move || {
        while !stop.load(Ordering::Acquire) {
            let enabled = go.lock().expect("editor gate");
            if *enabled {
                while let Some(job) = inbox.take() {
                    done.fetch_add(1, Ordering::AcqRel);
                    let (m, c) = (job.method.clone(), job.client.clone());
                    job.answer(Ok(json!({"did": m, "client": c})));
                }
            }
            drop(enabled);
            std::thread::sleep(Duration::from_millis(5));
        }
    })
}

#[test]
fn the_bridge_serves_mcp_and_survives_its_instance_and_its_client() {
    let t = tempfile::tempdir().expect("tempdir");
    let dir = t.path().join("agents");
    // None open: the agent is told how to allow agents; the bridge itself answers.
    let mut b = Bridge::start(&dir);
    let hi = b.ask(
        "initialize",
        json!({"protocolVersion": "2025-06-18", "clientInfo": {"name": "bridge-test"}}),
    );
    assert_eq!(hi["result"]["protocolVersion"], json!("2025-06-18"));
    let tools = b.ask("tools/list", json!({}))["result"]["tools"].clone();
    assert!(tools.as_array().expect("tools").len() >= 25);
    let r = b.tool("ca72_describe", json!({}));
    assert_eq!(r["isError"], json!(true));
    assert!(
        r["structuredContent"]["error"]
            .as_str()
            .unwrap_or("")
            .contains("AGENTS")
    );
    // One open: calls reach it, named by the client.
    let server = Server::start_in(&dir, "standalone", "standalone-1").expect("server");
    let (go, stop, done) = (
        Arc::new(Mutex::new(true)),
        Arc::new(AtomicBool::new(false)),
        Arc::new(AtomicUsize::new(0)),
    );
    let ed = editor(
        &server,
        Arc::clone(&go),
        Arc::clone(&stop),
        Arc::clone(&done),
    );
    let r = b.tool("ca72_set", json!({"values": {"cutoff": 1}}));
    assert_eq!(r["isError"], json!(false), "{r}");
    assert_eq!(r["structuredContent"]["did"], json!("parameters.set"));
    assert_eq!(r["structuredContent"]["client"], json!("bridge-test"));
    let list = b.tool("ca72_instances", json!({}));
    assert_eq!(
        list["structuredContent"]["instances"][0]["id"],
        json!("standalone-1")
    );
    // The bridge killed while its request waits: the instance drops it and serves the next.
    *go.lock().expect("pause editor") = false;
    let before = done.load(Ordering::Acquire);
    b.send(
        "tools/call",
        json!({"name": "ca72_describe", "arguments": {}}),
    );
    std::thread::sleep(Duration::from_millis(200));
    b.child.kill().expect("kill");
    b.child.wait().expect("wait");
    // The server must have observed the close and marked its pending request
    // dropped before the editor resumes. A fixed sleep cannot establish that
    // when a loaded test machine delays the connection thread.
    let deadline = Instant::now() + Duration::from_secs(5);
    while server.clients() != 0 {
        assert!(
            Instant::now() < deadline,
            "the server did not notice the close"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
    *go.lock().expect("resume editor") = true;
    let mut b = Bridge::start(&dir);
    let r = b.tool("ca72_describe", json!({}));
    assert_eq!(r["structuredContent"]["did"], json!("describe"));
    assert_eq!(
        done.load(Ordering::Acquire),
        before + 1,
        "the dropped request was carried out as well as the new request"
    );
    // The instance closes: said; the bridge ends when its input does.
    stop.store(true, Ordering::Release);
    ed.join().expect("editor");
    drop(server);
    let r = b.tool("ca72_describe", json!({}));
    assert_eq!(r["isError"], json!(true));
    drop(b.input.take());
    let began = Instant::now();
    let status = loop {
        if let Some(s) = b.child.try_wait().expect("wait") {
            break s;
        }
        assert!(
            began.elapsed() < Duration::from_secs(5),
            "the bridge did not end"
        );
        std::thread::sleep(Duration::from_millis(10));
    };
    assert!(status.success());
}

#[test]
fn the_bridge_says_how_it_is_used() {
    let help = Command::new(env!("CARGO_BIN_EXE_ca72-mcp"))
        .arg("--help")
        .output()
        .expect("ca72-mcp");
    assert!(help.status.success());
    assert!(String::from_utf8_lossy(&help.stdout).contains("AGENTS"));
    let bad = Command::new(env!("CARGO_BIN_EXE_ca72-mcp"))
        .arg("--nonsense")
        .output()
        .expect("ca72-mcp");
    assert_eq!(bad.status.code(), Some(2));
}
