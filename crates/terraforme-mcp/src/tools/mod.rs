//! Ferramentas MCP expostas pelo daemon.

pub mod messaging;
pub mod spawn_subagent;

use serde_json::Value;

/// POST JSON para `http://{addr}{path}` usando TCP cru (sem dependência extra).
pub async fn http_post_json(addr: &str, path: &str, body: &Value) -> anyhow::Result<()> {
    use tokio::io::AsyncWriteExt;
    use tokio::net::TcpStream;

    let body_str = serde_json::to_string(body)?;
    let request = format!(
        "POST {} HTTP/1.0\r\nHost: {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
        path,
        addr,
        body_str.len(),
        body_str
    );

    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(request.as_bytes()).await?;
    stream.shutdown().await?;
    Ok(())
}
