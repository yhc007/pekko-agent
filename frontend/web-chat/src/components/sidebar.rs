use yew::prelude::*;

use crate::types::AgentMeta;

#[derive(Properties, PartialEq)]
pub struct SidebarProps {
    pub agents: Vec<AgentMeta>,
    pub selected_agent: Option<String>,
    pub on_select_agent: Callback<String>,
    pub on_new_chat: Callback<()>,
    pub health_status: Option<String>,
}

#[function_component(Sidebar)]
pub fn sidebar(props: &SidebarProps) -> Html {
    let health_class = match props.health_status.as_deref() {
        Some("healthy") => "healthy",
        Some(_) => "unhealthy",
        None => "unknown",
    };
    let health_text = match props.health_status.as_deref() {
        Some("healthy") => "서버 연결됨",
        Some(_) => "서버 오류",
        None => "연결 확인 중...",
    };

    html! {
        <div class="sidebar">
            // ── Header ──
            <div class="sidebar-header">
                <h1>
                    <span class="dot"></span>
                    { "Pekko Agent" }
                </h1>
                <p>{ "EHS 안전보건환경 AI" }</p>
            </div>

            // ── Agents ──
            <div class="sidebar-section">
                <div class="sidebar-section-title">{ "에이전트" }</div>
                { for props.agents.iter().map(|agent| {
                    let is_active = props.selected_agent.as_ref() == Some(&agent.id);
                    let agent_id = agent.id.clone();
                    let on_click = {
                        let cb = props.on_select_agent.clone();
                        Callback::from(move |_: MouseEvent| cb.emit(agent_id.clone()))
                    };
                    html! {
                        <div
                            class={classes!("agent-item", is_active.then_some("active"))}
                            onclick={on_click}
                        >
                            <div class={classes!("agent-icon", agent.css_class.to_string())}>
                                { agent.icon }
                            </div>
                            <div>
                                <div class="agent-name">{ &agent.name }</div>
                                <div class="agent-desc">{ &agent.description }</div>
                            </div>
                        </div>
                    }
                })}
            </div>

            // ── New chat button ──
            <div class="sidebar-section">
                <div class="new-chat-btn" onclick={
                    let cb = props.on_new_chat.clone();
                    Callback::from(move |_: MouseEvent| cb.emit(()))
                }>
                    { "+ 새 대화" }
                </div>
            </div>

            // ── Footer ──
            <div class="sidebar-footer">
                <span class={classes!("status-dot", health_class)}></span>
                { health_text }
            </div>
        </div>
    }
}
