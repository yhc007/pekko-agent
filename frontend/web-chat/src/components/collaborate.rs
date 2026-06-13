use yew::prelude::*;

use crate::markdown;
use crate::types::{AgentMeta, CollabAgentResponse, CollaborationResult};

#[derive(Properties, PartialEq)]
pub struct CollaborateResultProps {
    pub result: CollaborationResult,
    pub agents: Vec<AgentMeta>,
}

fn agent_card(ar: &CollabAgentResponse, agents: &[AgentMeta]) -> Html {
    let meta = agents.iter().find(|a| a.id == ar.agent_id);
    let name = meta.map(|m| m.name.as_str()).unwrap_or(ar.agent_id.as_str());
    let css = meta.map(|m| m.css_class).unwrap_or("permit");
    let total_tokens = ar.input_tokens + ar.output_tokens;

    let body = if let Some(ref err) = ar.error {
        html! { <div class="collab-agent-card-error">{ format!("오류: {}", err) }</div> }
    } else {
        let rendered = markdown::markdown_to_html(&ar.response);
        Html::from_html_unchecked(AttrValue::from(
            format!("<div class=\"collab-agent-card-body md-content\">{rendered}</div>"),
        ))
    };

    html! {
        <div class="collab-agent-card">
            <div class="collab-agent-card-header">
                <div class={classes!("agent-icon", css.to_string())}></div>
                <span class="collab-agent-card-name">{ name }</span>
                <span class="collab-token-badge">{ format!("{}t", total_tokens) }</span>
            </div>
            { body }
        </div>
    }
}

#[function_component(CollaborateResultView)]
pub fn collaborate_result_view(props: &CollaborateResultProps) -> Html {
    let synthesis_html = {
        let rendered = markdown::markdown_to_html(&props.result.synthesis);
        Html::from_html_unchecked(AttrValue::from(
            format!("<div class=\"collab-synthesis-body md-content\">{rendered}</div>"),
        ))
    };

    html! {
        <div class="collab-result">
            <div class="collab-agents">
                { for props.result.agent_responses.iter().map(|ar| agent_card(ar, &props.agents)) }
            </div>
            <div class="collab-synthesis">
                <div class="collab-synthesis-header">{ "종합 분석" }</div>
                { synthesis_html }
                <div class="token-info" style="margin-top:8px;">
                    { format!("총 토큰: {} → {}", props.result.total_in_tokens, props.result.total_out_tokens) }
                </div>
            </div>
        </div>
    }
}
