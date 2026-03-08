use yew::prelude::*;

use crate::types::AgentMeta;

#[derive(Properties, PartialEq)]
pub struct HeaderProps {
    pub agent: Option<AgentMeta>,
}

#[function_component(ChatHeader)]
pub fn chat_header(props: &HeaderProps) -> Html {
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
