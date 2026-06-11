mod api;
mod components;
pub mod markdown;
mod types;

use std::cell::RefCell;
use std::rc::Rc;
use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use components::chat::ChatArea;
use components::header::ChatHeader;
use components::sidebar::Sidebar;
use types::{AgentMeta, ChatMessage, MessageRole, StreamEvent, TokenUsage};

// Transient state collected while streaming a response
#[derive(Default)]
struct StreamState {
    accumulated_text: String,
    session_id: Option<Uuid>,
    tools_used: Vec<String>,
    input_tokens: u32,
    output_tokens: u32,
    error: Option<String>,
}

#[function_component(App)]
fn app() -> Html {
    // ── State ──
    let agents = use_state(|| AgentMeta::defaults());
    let selected_agent_id = use_state(|| Option::<String>::None);
    let messages = use_state(Vec::<ChatMessage>::new);
    let session_id = use_state(|| Option::<Uuid>::None);
    let is_loading = use_state(|| false);
    let health_status = use_state(|| Option::<String>::None);
    let streaming_status = use_state(|| Option::<String>::None);
    let streaming_text = use_state(|| String::new());

    // ── Health check on mount ──
    {
        let health_status = health_status.clone();
        use_effect_with((), move |_| {
            spawn_local(async move {
                match api::fetch_health().await {
                    Ok(h) => health_status.set(Some(h.status)),
                    Err(_) => health_status.set(Some("error".into())),
                }
            });
            || ()
        });
    }

    // ── Callbacks ──
    let on_select_agent = {
        let selected_agent_id = selected_agent_id.clone();
        let messages = messages.clone();
        let session_id = session_id.clone();
        Callback::from(move |id: String| {
            selected_agent_id.set(Some(id));
            messages.set(vec![]);
            session_id.set(None);
        })
    };

    let on_new_chat = {
        let messages = messages.clone();
        let session_id = session_id.clone();
        Callback::from(move |_: ()| {
            messages.set(vec![]);
            session_id.set(None);
        })
    };

    let on_send = {
        let messages = messages.clone();
        let session_id = session_id.clone();
        let is_loading = is_loading.clone();
        let streaming_status = streaming_status.clone();
        let streaming_text = streaming_text.clone();
        let selected_agent_id = selected_agent_id.clone();
        Callback::from(move |content: String| {
            let agent_id = match (*selected_agent_id).clone() {
                Some(id) => id,
                None => return,
            };

            // Add user message immediately
            let user_msg = ChatMessage {
                id: Uuid::new_v4().to_string(),
                role: MessageRole::User,
                content: content.clone(),
                tools_used: vec![],
                token_usage: None,
                timestamp: chrono::Utc::now(),
            };
            let mut new_msgs = (*messages).clone();
            new_msgs.push(user_msg);
            messages.set(new_msgs);
            is_loading.set(true);
            streaming_status.set(None);
            streaming_text.set(String::new());

            // Clone handles for async block
            let messages = messages.clone();
            let session_id = session_id.clone();
            let is_loading = is_loading.clone();
            let streaming_status = streaming_status.clone();
            let streaming_text = streaming_text.clone();
            let sid = (*session_id).clone();

            spawn_local(async move {
                // Shared mutable state for the streaming callback
                let stream_state: Rc<RefCell<StreamState>> =
                    Rc::new(RefCell::new(StreamState::default()));

                // Clones for the callback (moved in)
                let ss_cb = stream_state.clone();
                let st_text_cb = streaming_text.clone();
                let st_status_cb = streaming_status.clone();

                let result = api::stream_query(
                    &agent_id,
                    &content,
                    sid,
                    move |event| match event {
                        StreamEvent::Thinking { round } => {
                            st_status_cb.set(Some(format!(
                                "AI 분석 중... ({}회차)",
                                round + 1
                            )));
                        }
                        StreamEvent::ToolUse { tool, .. } => {
                            st_status_cb.set(Some(format!("🔧 {} 실행 중...", tool)));
                        }
                        StreamEvent::ToolResult { tool, ok } => {
                            st_status_cb.set(Some(if ok {
                                format!("✓ {} 완료", tool)
                            } else {
                                format!("✗ {} 실패", tool)
                            }));
                        }
                        StreamEvent::TextChunk { text } => {
                            let mut s = ss_cb.borrow_mut();
                            s.accumulated_text.push_str(&text);
                            st_text_cb.set(s.accumulated_text.clone());
                            st_status_cb.set(Some("응답 작성 중...".into()));
                        }
                        StreamEvent::Done {
                            session_id: sid,
                            tools_used,
                            input_tokens,
                            output_tokens,
                        } => {
                            let mut s = ss_cb.borrow_mut();
                            s.session_id = Some(sid);
                            s.tools_used = tools_used;
                            s.input_tokens = input_tokens;
                            s.output_tokens = output_tokens;
                        }
                        StreamEvent::Error { message } => {
                            ss_cb.borrow_mut().error = Some(message);
                        }
                    },
                )
                .await;

                // Stream ended — clear transient UI state
                streaming_text.set(String::new());
                streaming_status.set(None);
                is_loading.set(false);

                // Handle transport error (network/parse failures)
                if let Err(e) = result {
                    let err_msg = ChatMessage {
                        id: Uuid::new_v4().to_string(),
                        role: MessageRole::System,
                        content: format!("네트워크 오류: {e}"),
                        tools_used: vec![],
                        token_usage: None,
                        timestamp: chrono::Utc::now(),
                    };
                    let mut msgs = (*messages).clone();
                    msgs.push(err_msg);
                    messages.set(msgs);
                    return;
                }

                let state = stream_state.borrow();

                // Handle application-level error from the stream
                if let Some(ref err) = state.error {
                    let err_msg = ChatMessage {
                        id: Uuid::new_v4().to_string(),
                        role: MessageRole::System,
                        content: format!("오류: {err}"),
                        tools_used: vec![],
                        token_usage: None,
                        timestamp: chrono::Utc::now(),
                    };
                    let mut msgs = (*messages).clone();
                    msgs.push(err_msg);
                    messages.set(msgs);
                    return;
                }

                // Success — persist session_id and add final message
                if let Some(sid) = state.session_id {
                    session_id.set(Some(sid));
                }

                let assistant_msg = ChatMessage {
                    id: Uuid::new_v4().to_string(),
                    role: MessageRole::Assistant,
                    content: state.accumulated_text.clone(),
                    tools_used: state.tools_used.clone(),
                    token_usage: Some(TokenUsage {
                        input_tokens: state.input_tokens,
                        output_tokens: state.output_tokens,
                    }),
                    timestamp: chrono::Utc::now(),
                };
                let mut msgs = (*messages).clone();
                msgs.push(assistant_msg);
                messages.set(msgs);
            });
        })
    };

    let on_example_click = on_send.clone();

    // ── Resolve selected agent meta ──
    let selected_meta = (*selected_agent_id).as_ref().and_then(|id| {
        agents.iter().find(|a| a.id == *id).cloned()
    });

    html! {
        <>
            <Sidebar
                agents={(*agents).clone()}
                selected_agent={(*selected_agent_id).clone()}
                on_select_agent={on_select_agent}
                on_new_chat={on_new_chat}
                health_status={(*health_status).clone()}
            />
            <div class="main">
                <ChatHeader agent={selected_meta.clone()} />
                <ChatArea
                    messages={(*messages).clone()}
                    is_loading={*is_loading}
                    streaming_status={(*streaming_status).clone()}
                    streaming_text={(*streaming_text).clone()}
                    on_send={on_send}
                    on_example_click={on_example_click}
                    has_agent={selected_meta.is_some()}
                />
            </div>
        </>
    }
}

fn main() {
    wasm_logger::init(wasm_logger::Config::default());
    yew::Renderer::<App>::new().render();
}
