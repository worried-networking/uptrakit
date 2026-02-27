use crate::client::authenticated_client;
use crate::error::Result;
use futures_util::StreamExt;
use rootcause::prelude::*;
use uptrakit_openapi_client::Uuid;
use uptrakit_openapi_client::update_output_stream::UpdateOutputEvent;

/// Parameters for tailing update output.
pub struct TailParams<'a> {
    pub update_history_id: &'a Uuid,
    pub server: Option<&'a str>,
    pub token: Option<&'a str>,
    pub insecure: bool,
}

/// Result of a tail operation.
pub struct TailResult {
    /// Final status of the update ("completed" or "failed").
    pub status: String,
    /// Error message if the update failed.
    pub error: Option<String>,
}

impl TailResult {
    /// Map the final update status to a CLI exit code.
    ///
    /// - `completed` → 0
    /// - `failed` → 1
    /// - anything else → 2
    pub fn exit_code(&self) -> i32 {
        match self.status.as_str() {
            "completed" => 0,
            "failed" => 1,
            _ => 2,
        }
    }
}

/// Connect to the update output SSE stream and print lines to stdout.
///
/// ANSI escape codes pass through natively to the terminal. Status changes
/// are printed to stderr. Returns when the update completes or the stream
/// closes. Ctrl+C detaches without aborting the update.
pub async fn tail(params: TailParams<'_>) -> Result<TailResult> {
    // For SSE we don't want a request timeout — the connection is long-lived.
    let client = authenticated_client(params.server, params.token, params.insecure, None)?;

    eprintln!("Tailing update output for {} ...", params.update_history_id);

    let stream = client
        .stream_update_output(params.update_history_id)
        .await
        .context_to()?;
    tokio::pin!(stream);

    let result = loop {
        tokio::select! {
            biased;
            _ = tokio::signal::ctrl_c() => {
                eprintln!("\nDetached (update continues in the background).");
                break TailResult {
                    status: "detached".to_string(),
                    error: None,
                };
            }
            event = stream.next() => {
                match event {
                    Some(Ok(UpdateOutputEvent::Output(line))) => {
                        print!("{}", line.text);
                    }
                    Some(Ok(UpdateOutputEvent::Completed(completed))) => {
                        eprintln!("Update {}", completed.status);
                        if let Some(ref err) = completed.error {
                            eprintln!("Error: {err}");
                        }
                        break TailResult {
                            status: completed.status,
                            error: completed.error,
                        };
                    }
                    Some(Err(e)) => {
                        eprintln!("Stream error: {e}");
                        break TailResult {
                            status: "error".to_string(),
                            error: Some(e.to_string()),
                        };
                    }
                    None => {
                        eprintln!("Stream ended without completion event.");
                        break TailResult {
                            status: "disconnected".to_string(),
                            error: None,
                        };
                    }
                }
            }
        }
    };

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exit_code_completed_is_zero() {
        let r = TailResult {
            status: "completed".to_string(),
            error: None,
        };
        assert_eq!(r.exit_code(), 0);
    }

    #[test]
    fn exit_code_failed_is_one() {
        let r = TailResult {
            status: "failed".to_string(),
            error: Some("boom".to_string()),
        };
        assert_eq!(r.exit_code(), 1);
    }

    #[test]
    fn exit_code_other_is_two() {
        let r = TailResult {
            status: "detached".to_string(),
            error: None,
        };
        assert_eq!(r.exit_code(), 2);
    }
}
