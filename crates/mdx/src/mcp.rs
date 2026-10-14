//! Stdio MCP adapter. Analysis stays in the same stateless operation layer as CLI.
use crate::{
    context::Context,
    envelope::{Envelope, Exit},
    operations,
};
use rmcp::{
    model::{
        CallToolRequestParams, CallToolResponse, CallToolResult, ErrorCode, Implementation,
        ListToolsResult, PaginatedRequestParams, ProgressNotificationParam, ServerCapabilities,
        ServerConfig, Tool, ToolAnnotations,
    },
    service::{RequestContext, RoleServer},
    ErrorData as McpError, ServerHandler, ServiceExt,
};
use serde_json::Value;
use std::{collections::BTreeMap, sync::Arc};
use tokio::sync::{watch, Semaphore};

struct CancelOnDrop(tokio_util::sync::CancellationToken);
impl Drop for CancelOnDrop {
    fn drop(&mut self) {
        self.0.cancel();
    }
}

// A dedicated bounded stdin pump observes physical EOF independently of RMCP's
// handler draining and avoids Tokio's non-cancellable blocking stdin reads.
struct StdinPipe {
    receiver: tokio::sync::mpsc::Receiver<Vec<u8>>,
    current: Vec<u8>,
    position: usize,
}
impl tokio::io::AsyncRead for StdinPipe {
    fn poll_read(
        self: std::pin::Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
        buffer: &mut tokio::io::ReadBuf<'_>,
    ) -> std::task::Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if buffer.remaining() == 0 {
            return std::task::Poll::Ready(Ok(()));
        }
        loop {
            if this.position < this.current.len() {
                let count = (this.current.len() - this.position).min(buffer.remaining());
                buffer.put_slice(&this.current[this.position..this.position + count]);
                this.position += count;
                return std::task::Poll::Ready(Ok(()));
            }
            match this.receiver.poll_recv(cx) {
                std::task::Poll::Ready(Some(bytes)) => {
                    this.current = bytes;
                    this.position = 0;
                }
                std::task::Poll::Ready(None) => return std::task::Poll::Ready(Ok(())),
                std::task::Poll::Pending => return std::task::Poll::Pending,
            }
        }
    }
}

fn stdin_pipe(shutdown: tokio_util::sync::CancellationToken) -> StdinPipe {
    let (sender, receiver) = tokio::sync::mpsc::channel(8);
    std::thread::spawn(move || {
        use std::io::Read;
        let stdin = std::io::stdin();
        let mut input = stdin.lock();
        let mut buffer = [0u8; 8192];
        let mut frame_bytes = 0;
        loop {
            let count = match input.read(&mut buffer) {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            };
            for byte in &buffer[..count] {
                frame_bytes += 1;
                if frame_bytes > 2 * 1024 * 1024 {
                    eprintln!("mdx MCP: inbound JSON-RPC frame exceeds 2 MiB");
                    shutdown.cancel();
                    return;
                }
                if *byte == b'\n' {
                    frame_bytes = 0;
                }
            }
            if sender.blocking_send(buffer[..count].to_vec()).is_err() {
                break;
            }
        }
        shutdown.cancel();
    });
    StdinPipe {
        receiver,
        current: Vec::new(),
        position: 0,
    }
}

#[derive(Clone)]
pub struct MdxServer {
    context: Context,
    tools: Arc<Vec<Tool>>,
    names: Arc<BTreeMap<String, String>>,
    slots: Arc<Semaphore>,
}

impl MdxServer {
    pub fn new(context: Context, jobs: usize) -> Result<Self, crate::envelope::Failure> {
        if !(1..=8).contains(&jobs) {
            return Err(crate::artifacts::failure(
                "invalid_arguments",
                "MCP jobs must be 1..8",
            ));
        }
        let mut tools = Vec::new();
        let mut names = BTreeMap::new();
        for descriptor in operations::descriptors() {
            names.insert(descriptor.mcp_name.clone(), descriptor.name);
            let tool = Tool::new(
                descriptor.mcp_name,
                descriptor.description,
                Arc::new(descriptor.input_schema.as_object().unwrap().clone()),
            )
            .with_raw_output_schema(Arc::new(
                descriptor.output_schema.as_object().unwrap().clone(),
            ))
            .with_annotations(ToolAnnotations::from_raw(
                None,
                Some(false),
                Some(false),
                None,
                Some(false),
            ));
            tools.push(tool);
        }
        Ok(Self {
            context,
            tools: Arc::new(tools),
            names: Arc::new(names),
            slots: Arc::new(Semaphore::new(jobs)),
        })
    }
}

fn result(response: Envelope<Value>) -> CallToolResponse {
    let error = matches!(response.exit, Exit::Failed | Exit::Usage);
    let value = serde_json::to_value(response).unwrap();
    if error {
        CallToolResult::structured_error(value).into()
    } else {
        CallToolResult::structured(value).into()
    }
}

impl ServerHandler for MdxServer {
    fn get_info(&self) -> ServerConfig {
        ServerConfig::new(ServerCapabilities::builder().enable_tools().build())
            .with_server_info(Implementation::new("mdx",env!("CARGO_PKG_VERSION")))
            .with_instructions("Stateless Mega Drive analysis. Pass project_root as the calling agent's absolute workspace directory on every call: relative inputs/outputs and the default .mdx/artifacts store resolve there. artifact_store optionally selects a different per-call store; use the same project_root/artifact_store for sha256 ID retrieval. Concurrent calls never change another call's paths. Without project_root, the launch context is retained. All results carry coverage and limits. ROM/snapshot sources accept local paths or sha256: IDs. Use artifact_import, boot, memory_read/search/diff, trace/watch, render/graphics and survey. Large results are immutable JSON artifacts: select JSON Pointers or record pages with artifact_view, or exact byte pages with artifact_read. snapshot_artifact contains captured buffers for render/memory tools. boot/trace/watch/ram_dump accept from_state (machine-state ID or path) and save_state:true to publish state_artifact. Run frame targets, probe start_frame and replay events are relative to the loaded state; absolute frame numbers remain in results. States require the exact ROM and compatible engine; no server session is retained. Partial evidence is not whole-game or cycle-equivalence proof.")
    }

    fn get_tool(&self, name: &str) -> Option<Tool> {
        self.tools.iter().find(|tool| tool.name == name).cloned()
    }

    async fn list_tools(
        &self,
        request: Option<PaginatedRequestParams>,
        _context: RequestContext<RoleServer>,
    ) -> Result<ListToolsResult, McpError> {
        if request.is_some_and(|r| r.cursor.is_some()) {
            return Err(McpError::invalid_params(
                "tool list is complete and has no pagination cursor",
                None,
            ));
        }
        let mut result = ListToolsResult::default();
        result.tools = self.tools.as_ref().clone();
        Ok(result)
    }

    async fn call_tool(
        &self,
        request: CallToolRequestParams,
        request_context: RequestContext<RoleServer>,
    ) -> Result<CallToolResponse, McpError> {
        let name = self
            .names
            .get(request.name.as_ref())
            .cloned()
            .ok_or_else(|| McpError::new(ErrorCode::METHOD_NOT_FOUND, "unknown mdx tool", None))?;
        let arguments = Value::Object(request.arguments.unwrap_or_default());
        if serde_json::to_vec(&arguments).unwrap().len() > 1024 * 1024 {
            return Ok(result(Envelope::failure(
                &format!("mdx.{name}"),
                crate::artifacts::failure("limit_exceeded", "tool arguments exceed 1 MiB"),
            )));
        }
        let permit = tokio::select! {
            permit=self.slots.clone().acquire_owned()=> permit.map_err(|_|McpError::internal_error("execution queue closed",None))?,
            _=request_context.ct.cancelled()=> return Ok(result(Envelope::failure(&format!("mdx.{name}"),crate::artifacts::failure("cancelled","cancelled while waiting for an execution slot")))),
            _=self.context.cancellation.cancelled()=> return Ok(result(Envelope::failure(&format!("mdx.{name}"),crate::artifacts::failure("cancelled","transport closed while waiting for an execution slot")))),
        };
        let token = request_context.meta.get_progress_token();
        let (progress_sender, mut progress_receiver) = watch::channel((0u64, 0u64));
        let mut context = self.context.clone();
        context.cancellation = self.context.cancellation.child_token();
        let worker_cancellation = context.cancellation.clone();
        let _cancel_on_drop = CancelOnDrop(context.cancellation.clone());
        context.progress = Some(Arc::new(move |completed, total| {
            let _ = progress_sender.send((completed, total));
        }));
        if let Some(token) = token.clone() {
            let _ = request_context
                .peer
                .notify_progress(
                    ProgressNotificationParam::new(token, 0.0).with_message("mdx analysis started"),
                )
                .await;
        }
        let worker_name = name.clone();
        let mut worker = tokio::task::spawn_blocking(move || {
            let _permit = permit;
            operations::execute(&worker_name, arguments, &context)
        });
        let mut progress_open = token.is_some();
        let mut request_open = true;
        let response = loop {
            tokio::select! {
                finished=&mut worker=>break finished,
                _=request_context.ct.cancelled(),if request_open=> {worker_cancellation.cancel();request_open=false;},
                changed=progress_receiver.changed(),if progress_open=> {
                    if changed.is_err() {progress_open=false;continue;}
                    let (completed,total)=*progress_receiver.borrow_and_update();
                    if let Some(token)=token.clone() {
                        let notification=ProgressNotificationParam::new(token,completed as f64).with_total(total as f64);
                        let _=request_context.peer.notify_progress(notification).await;
                    }
                }
            }
        };
        Ok(result(response.unwrap_or_else(|error| {
            Envelope::failure(
                &format!("mdx.{name}"),
                crate::artifacts::failure("analysis_worker_failed", error.to_string()),
            )
        })))
    }
}

pub async fn serve(context: Context, jobs: usize) -> Result<(), Box<dyn std::error::Error>> {
    let shutdown = context.cancellation.clone();
    let server =
        MdxServer::new(context, jobs).map_err(|failure| std::io::Error::other(failure.message))?;
    let input = stdin_pipe(shutdown.clone());
    let running = server
        .serve_with_ct((input, tokio::io::stdout()), shutdown.clone())
        .await?;
    let result = running.waiting().await;
    shutdown.cancel();
    result?;
    Ok(())
}
