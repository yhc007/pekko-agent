mod api;
mod components;
pub mod markdown;
mod types;

use uuid::Uuid;
use wasm_bindgen_futures::spawn_local;
use yew::prelude::*;

use components::chat::ChatArea;
use components::header::ChatHeader;
use components::sidebar::Sidebar;
use types::{AgentMeta, ChatMessage, MessageRole};

#[function_component(App)]
fn app() -> Html {
    // ── State ──
    let agents = use_state(|| AgentMeta::defaults());
    let selected_agent_id = use_state(|| Option::<String>::None);
    let messages = use_state(Vec::<ChatMessage>::new);
    let session_id = use_state(|| Option::<Uuid>::None);
    let is_loading = use_state(|| false);
    let health_status = use_state(|| Option::<String>::None);

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
        let selected_agent_id = selected_agent_id.clone();
        Callback::from(move |content: String| {
            let agent_id = match (*selected_agent_id).clone() {
                Some(id) => id,
                None => return,
            };

            // Add user message
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

            let messages = messages.clone();
            let session_id = session_id.clone();
            let is_loading = is_loading.clone();
            let sid = (*session_id).clone();

            spawn_local(async move {
                match api::send_query(&agent_id, &content, sid).await {
                    Ok(resp) => {
                        session_id.set(Some(resp.session_id));
                        let assistant_msg = ChatMessage {
                            id: Uuid::new_v4().to_string(),
                            role: MessageRole::Assistant,
                            content: resp.response,
                            tools_used: resp.tools_used,
                            token_usage: Some(resp.token_usage),
                            timestamp: chrono::Utc::now(),
                        };
                        let mut msgs = (*messages).clone();
                        msgs.push(assistant_msg);
                        messages.set(msgs);
                    }
                    Err(e) => {
                        let err_msg = ChatMessage {
                            id: Uuid::new_v4().to_string(),
                            role: MessageRole::System,
                            content: format!("오류: {e}"),
                            tools_used: vec![],
                            token_usage: None,
                            timestamp: chrono::Utc::now(),
                        };
                        let mut msgs = (*messages).clone();
                        msgs.push(err_msg);
                        messages.set(msgs);
                    }
                }
                is_loading.set(false);
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
