#![cfg(windows)]

use std::process::Stdio;
use std::time::Duration;

use devitd_client::DevitClient;
use tokio::process::Command;
use tokio::time::sleep;

fn find_devitd_binary() -> std::path::PathBuf {
    let exe = std::env::current_exe().expect("current_exe");
    let target_dir = exe
        .parent() // deps/
        .and_then(|p| p.parent()) // debug/
        .expect("target debug dir");
    let candidate = target_dir.join("devitd.exe");
    if candidate.is_file() {
        return candidate;
    }
    // Fallback to workspace-root target
    let ws_candidate = target_dir
        .parent()
        .map(|p| p.join("debug").join("devitd.exe"))
        .unwrap_or(candidate.clone());
    ws_candidate
}

#[tokio::test]
async fn pipes_smoke_connects() {
    let secret = "test-secret";
    let pipe_name = format!(
        "devitd-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );

    let devitd_path = find_devitd_binary();
    let mut child = Command::new(&devitd_path)
        .arg("--socket")
        .arg(format!("pipe:{}", pipe_name))
        .arg("--secret")
        .arg(secret)
        .arg("--debug")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn devitd");

    // Wait until daemon is ready
    let mut connected = false;
    for _ in 0..30 {
        // ~6s
        match DevitClient::connect(&format!("pipe:{}", pipe_name), "test", secret).await {
            Ok(_) => {
                connected = true;
                break;
            }
            Err(_) => sleep(Duration::from_millis(200)).await,
        }
    }

    // Clean up
    let _ = child.kill().await;

    assert!(connected, "devitd should accept named pipe connection");
}

#[tokio::test]
async fn tcp_fallback_connects() {
    let secret = "test-secret";

    // Reserve an ephemeral port
    let sock = std::net::TcpListener::bind("127.0.0.1:0").expect("bind 0");
    let port = sock.local_addr().unwrap().port();
    drop(sock);

    let addr = format!("127.0.0.1:{}", port);
    let devitd_path = find_devitd_binary();
    let mut child = Command::new(&devitd_path)
        .arg("--socket")
        .arg(&addr)
        .arg("--secret")
        .arg(secret)
        .arg("--debug")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn devitd");

    // Wait until daemon is ready
    let mut connected = false;
    for _ in 0..30 {
        // ~6s
        match DevitClient::connect(&addr, "test", secret).await {
            Ok(_) => {
                connected = true;
                break;
            }
            Err(_) => sleep(Duration::from_millis(200)).await,
        }
    }

    let _ = child.kill().await;
    assert!(connected, "devitd should accept TCP connection");
}

// NOTE: ACL rejection with a different user is not trivially testable in CI.
// Provide an ignored test skeleton that can be enabled with an alternate-user
// launcher (DEVIT_ALT_USER_CMD) if the environment supports it.
#[tokio::test]
#[ignore]
async fn pipes_acl_rejects_other_user() {
    let secret = "test-secret";
    let pipe_name = format!(
        "devitd-acl-test-{}-{}",
        std::process::id(),
        chrono::Utc::now().timestamp_millis()
    );

    let devitd_path = find_devitd_binary();
    let mut child = Command::new(&devitd_path)
        .arg("--socket")
        .arg(format!("pipe:{}", pipe_name))
        .arg("--secret")
        .arg(secret)
        .arg("--debug")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn devitd");

    let alt = std::env::var("DEVIT_ALT_USER_CMD").ok();
    if alt.is_none() {
        let _ = child.kill().await;
        eprintln!("skipping: set DEVIT_ALT_USER_CMD to run this test with another user context");
        return;
    }

    // Try to connect from current user (should succeed)
    for _ in 0..20 {
        if DevitClient::connect(&format!("pipe:{}", pipe_name), "test", secret)
            .await
            .is_ok()
        {
            break;
        }
        sleep(Duration::from_millis(200)).await;
    }

    // Try to connect using alternate user launcher; command should fail with access denied
    let alt_cmd = alt.unwrap();
    let status = Command::new("cmd")
        .arg("/C")
        .arg(format!(
            "{} {}",
            alt_cmd,
            format!("{} {} {}", "devitd-client-test-connect", pipe_name, secret)
        ))
        .status()
        .await
        .expect("run alt user command");

    let _ = child.kill().await;

    assert!(
        !status.success(),
        "alternate user connect should fail due to ACL"
    );
}
