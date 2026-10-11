use async_trait::async_trait;
use pekko_agent_core::{Tool, ToolContext, ToolDefinition, ToolError, ToolOutput};
use serde::Deserialize;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::TcpStream;
use tracing::{info, warn};

/// CoreOS 로봇 제어 보드(ESP32 펌웨어) Tool
///
/// CoreOS 펌웨어의 TCP 명령 서버(기본 포트 7000)에 접속해
/// Tier 1 ↔ Tier 2 바이너리 프로토콜로 명령을 보냅니다.
/// 프레임 형식은 CoreOS `src/drivers/protocol.rs`와 동일합니다:
///   [STX][LEN_H][LEN_L][MSG_TYPE][PAYLOAD...][CRC16][ETX]
pub struct CoreOsRobotTool {
    /// 보드 주소 (예: "192.168.11.164:7000")
    addr: String,
    timeout: Duration,
}

impl CoreOsRobotTool {
    pub fn new(addr: impl Into<String>) -> Self {
        Self {
            addr: addr.into(),
            timeout: Duration::from_millis(TIMEOUT_MS),
        }
    }
}

const TIMEOUT_MS: u64 = 5_000;

#[derive(Deserialize)]
#[serde(rename_all = "snake_case")]
enum Command {
    Ping,
    Status,
    EmergencyStop,
}

#[derive(Deserialize)]
struct Params {
    command: Command,
}

#[async_trait]
impl Tool for CoreOsRobotTool {
    fn definition(&self) -> ToolDefinition {
        ToolDefinition {
            name: "coreos_robot".to_string(),
            description: "CoreOS 로봇 제어 보드(ESP32)와 통신합니다. \
                          ping: 연결 확인 및 왕복 지연 측정, \
                          status: 업타임/힙 메모리/E-Stop 상태 조회, \
                          emergency_stop: 긴급 정지 (보드 재부팅 전까지 해제되지 않음). \
                          모터/서보 제어는 아직 지원되지 않습니다."
                .to_string(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {
                    "command": {
                        "type": "string",
                        "enum": ["ping", "status", "emergency_stop"],
                        "description": "보낼 명령"
                    }
                },
                "required": ["command"]
            }),
            required_permissions: vec![],
            timeout_ms: TIMEOUT_MS,
            idempotent: false, // emergency_stop은 보드 상태를 바꿈
        }
    }

    async fn execute(
        &self,
        input: serde_json::Value,
        _ctx: &ToolContext,
    ) -> Result<ToolOutput, ToolError> {
        let params: Params = serde_json::from_value(input)
            .map_err(|e| ToolError::ValidationFailed(format!("파라미터 오류: {e}")))?;

        let (msg_type, payload): (u8, &[u8]) = match params.command {
            Command::Ping => (CMD_PING, b"pekko-agent"),
            Command::Status => (CMD_REQUEST_STATUS, b""),
            Command::EmergencyStop => (CMD_EMERGENCY_STOP, b""),
        };

        let start = Instant::now();
        let reply = tokio::time::timeout(self.timeout, request(&self.addr, msg_type, payload))
            .await
            .map_err(|_| ToolError::Timeout)?;
        let rtt_ms = start.elapsed().as_secs_f64() * 1000.0;

        let (reply_type, reply_payload) = match reply {
            Ok(r) => r,
            Err(e) => {
                warn!(addr = %self.addr, error = %e, "CoreOS 보드 통신 실패");
                return Ok(ToolOutput::error(format!("보드 통신 실패 ({}): {e}", self.addr)));
            }
        };

        info!(addr = %self.addr, msg_type, reply_type, rtt_ms, "CoreOS 명령 완료");
        Ok(to_output(reply_type, &reply_payload, rtt_ms))
    }
}

// ── 프로토콜 (CoreOS src/drivers/protocol.rs 와 동일해야 함) ──

const STX: u8 = 0x02;
const ETX: u8 = 0x03;
const MAX_PAYLOAD: usize = 256;

const CMD_EMERGENCY_STOP: u8 = 0x1F;
const CMD_REQUEST_STATUS: u8 = 0x20;
const CMD_PING: u8 = 0xFE;

const REPORT_SYSTEM_STATUS: u8 = 0x82;
const REPORT_ERROR_ALERT: u8 = 0x8F;
const REPORT_PONG: u8 = 0xFF;

/// CRC-16 CCITT (0xFFFF 초기값)
fn crc16_ccitt(data: &[u8]) -> u16 {
    let mut crc: u16 = 0xFFFF;
    for &byte in data {
        crc ^= (byte as u16) << 8;
        for _ in 0..8 {
            crc = if crc & 0x8000 != 0 { (crc << 1) ^ 0x1021 } else { crc << 1 };
        }
    }
    crc
}

fn encode_frame(msg_type: u8, payload: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(payload.len() + 7);
    buf.push(STX);
    buf.extend_from_slice(&(payload.len() as u16).to_be_bytes());
    buf.push(msg_type);
    buf.extend_from_slice(payload);
    let crc = crc16_ccitt(&buf[1..]);
    buf.extend_from_slice(&crc.to_be_bytes());
    buf.push(ETX);
    buf
}

/// 프레임 1개 송신 후 응답 프레임 1개 수신
async fn request(addr: &str, msg_type: u8, payload: &[u8]) -> anyhow::Result<(u8, Vec<u8>)> {
    let mut stream = TcpStream::connect(addr).await?;
    stream.write_all(&encode_frame(msg_type, payload)).await?;

    let mut header = [0u8; 4];
    stream.read_exact(&mut header).await?;
    anyhow::ensure!(header[0] == STX, "잘못된 응답 시작 바이트: 0x{:02x}", header[0]);
    let len = u16::from_be_bytes([header[1], header[2]]) as usize;
    anyhow::ensure!(len <= MAX_PAYLOAD, "응답 페이로드가 너무 큼: {len}");

    let mut rest = vec![0u8; len + 3];
    stream.read_exact(&mut rest).await?;
    let (body, trailer) = rest.split_at(len);
    anyhow::ensure!(trailer[2] == ETX, "잘못된 응답 끝 바이트");

    let mut crc_input = header[1..].to_vec();
    crc_input.extend_from_slice(body);
    let crc = u16::from_be_bytes([trailer[0], trailer[1]]);
    anyhow::ensure!(crc == crc16_ccitt(&crc_input), "응답 CRC 불일치");

    Ok((header[3], body.to_vec()))
}

fn to_output(reply_type: u8, payload: &[u8], rtt_ms: f64) -> ToolOutput {
    match reply_type {
        REPORT_PONG => ToolOutput::success(serde_json::json!({
            "reply": "pong",
            "echo": String::from_utf8_lossy(payload),
            "rtt_ms": rtt_ms,
        })),
        REPORT_SYSTEM_STATUS if payload.len() >= 13 => {
            let u32_at = |i: usize| u32::from_be_bytes([payload[i], payload[i + 1], payload[i + 2], payload[i + 3]]);
            ToolOutput::success(serde_json::json!({
                "reply": "system_status",
                "uptime_ms": u32_at(0),
                "heap_used": u32_at(4),
                "heap_free": u32_at(8),
                "estop": payload[12] != 0,
                "rtt_ms": rtt_ms,
            }))
        }
        REPORT_ERROR_ALERT if payload.len() >= 2 => {
            let code = match payload[0] {
                0x01 => "bad_frame",
                0x02 => "unknown_command",
                0x03 => "unsupported",
                _ => "unknown_error",
            };
            ToolOutput::error(format!("보드 에러: {code} (msg_type=0x{:02x})", payload[1]))
        }
        other => ToolOutput::error(format!(
            "예상하지 못한 응답: type=0x{other:02x}, {} bytes",
            payload.len()
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use tokio::net::TcpListener;

    fn ctx() -> ToolContext {
        ToolContext {
            tenant_id: "test".into(),
            user_id: "test".into(),
            session_id: uuid::Uuid::new_v4(),
            credentials: HashMap::new(),
            timeout: Duration::from_secs(5),
        }
    }

    #[test]
    fn test_encode_frame_layout() {
        // RequestStatus, 빈 페이로드: STX 00 00 20 CRC_H CRC_L ETX
        let frame = encode_frame(CMD_REQUEST_STATUS, b"");
        assert_eq!(frame.len(), 7);
        assert_eq!(&frame[..4], &[STX, 0x00, 0x00, 0x20]);
        assert_eq!(&frame[4..6], &crc16_ccitt(&[0x00, 0x00, 0x20]).to_be_bytes());
        assert_eq!(frame[6], ETX);
    }

    #[test]
    fn test_crc16_known_vector() {
        // CRC-16/CCITT-FALSE 표준 검증 벡터
        assert_eq!(crc16_ccitt(b"123456789"), 0x29B1);
    }

    /// 보드를 흉내 내는 서버: 요청 1개를 받아 고정 응답을 돌려줌
    async fn mock_board(reply: Vec<u8>) -> (String, tokio::task::JoinHandle<Vec<u8>>) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap().to_string();
        let handle = tokio::spawn(async move {
            let (mut sock, _) = listener.accept().await.unwrap();
            let mut header = [0u8; 4];
            sock.read_exact(&mut header).await.unwrap();
            let len = u16::from_be_bytes([header[1], header[2]]) as usize;
            let mut rest = vec![0u8; len + 3];
            sock.read_exact(&mut rest).await.unwrap();
            sock.write_all(&reply).await.unwrap();
            [header.to_vec(), rest].concat()
        });
        (addr, handle)
    }

    #[tokio::test]
    async fn test_status_parses_system_status() {
        let mut payload = Vec::new();
        payload.extend_from_slice(&9_700u32.to_be_bytes());
        payload.extend_from_slice(&43_312u32.to_be_bytes());
        payload.extend_from_slice(&30_416u32.to_be_bytes());
        payload.push(1);
        let (addr, board) = mock_board(encode_frame(REPORT_SYSTEM_STATUS, &payload)).await;

        let out = CoreOsRobotTool::new(addr)
            .execute(serde_json::json!({"command": "status"}), &ctx())
            .await
            .unwrap();

        assert!(!out.is_error);
        assert_eq!(out.content["uptime_ms"], 9_700);
        assert_eq!(out.content["heap_used"], 43_312);
        assert_eq!(out.content["heap_free"], 30_416);
        assert_eq!(out.content["estop"], true);
        assert_eq!(board.await.unwrap(), encode_frame(CMD_REQUEST_STATUS, b""));
    }

    #[tokio::test]
    async fn test_error_alert_becomes_tool_error_output() {
        let (addr, _board) = mock_board(encode_frame(REPORT_ERROR_ALERT, &[0x03, 0x10])).await;
        let out = CoreOsRobotTool::new(addr)
            .execute(serde_json::json!({"command": "ping"}), &ctx())
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content["error"].as_str().unwrap().contains("unsupported"));
    }

    #[tokio::test]
    async fn test_corrupted_reply_is_rejected() {
        let mut reply = encode_frame(REPORT_PONG, b"pekko-agent");
        let crc_idx = reply.len() - 2;
        reply[crc_idx] ^= 0xFF;
        let (addr, _board) = mock_board(reply).await;
        let out = CoreOsRobotTool::new(addr)
            .execute(serde_json::json!({"command": "ping"}), &ctx())
            .await
            .unwrap();
        assert!(out.is_error);
        assert!(out.content["error"].as_str().unwrap().contains("CRC"));
    }

    #[tokio::test]
    async fn test_invalid_command_is_validation_error() {
        let result = CoreOsRobotTool::new("127.0.0.1:1")
            .execute(serde_json::json!({"command": "set_motor"}), &ctx())
            .await;
        assert!(matches!(result, Err(ToolError::ValidationFailed(_))));
    }

    /// 실제 보드 대상 테스트: COREOS_ADDR=192.168.11.164:7000 cargo test -p pekko-agent-tools -- --ignored
    #[tokio::test]
    #[ignore]
    async fn live_board_ping_and_status() {
        let addr = std::env::var("COREOS_ADDR").expect("COREOS_ADDR not set");
        let tool = CoreOsRobotTool::new(addr);
        for command in ["ping", "status"] {
            let out = tool
                .execute(serde_json::json!({ "command": command }), &ctx())
                .await
                .unwrap();
            println!("{command}: {}", out.content);
            assert!(!out.is_error, "{command} failed: {}", out.content);
        }
    }
}
