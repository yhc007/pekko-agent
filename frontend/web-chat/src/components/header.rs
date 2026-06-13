use yew::prelude::*;

use crate::types::{AgentMeta, ViewMode};

#[derive(Properties, PartialEq)]
pub struct HeaderProps {
    pub agent: Option<AgentMeta>,
    pub view_mode: ViewMode,
}

#[function_component(ChatHeader)]
pub fn chat_header(props: &HeaderProps) -> Html {
    if props.view_mode == ViewMode::Collaborate {
        return html! {
            <div class="chat-header">
                <span style="font-size:20px;">{ "🤝" }</span>
                <span class="chat-header-title">{ "멀티에이전트 협업" }</span>
                <span class="chat-header-badge">{ "모든 EHS 에이전트가 협력하여 답변합니다" }</span>
            </div>
        };
    }

    if let Some(ref agent) = props.agent {
        html! {
            <div class="chat-header">
                <span style="font-size: 20px;">{ agent.icon }</span>
                <span class="chat-header-title">{ &agent.name }</span>
                <span class="chat-header-badge">{ &agent.description }</span>
            </div>
        }
    } else {
        html! {
            <div class="chat-header">
                <span class="chat-header-title">{ "EHS AI Agent" }</span>
                <span class="chat-header-badge">{ "에이전트를 선택하세요" }</span>
            </div>
        }
    }
}
