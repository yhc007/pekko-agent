#!/usr/bin/env python3
"""Post pekko-agent Agentic AI capabilities summary to Notion."""

import urllib.request
import urllib.parse
import json

NOTION_TOKEN = "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"
# "with SAM" page ID
PARENT_PAGE_ID = "31cdf49a-0d48-8117-b840-d963bf4659ae"

HEADERS = {
    "Authorization": f"Bearer {NOTION_TOKEN}",
    "Content-Type": "application/json",
    "Notion-Version": "2022-06-28",
}

def api_call(method, path, data=None):
    url = f"https://api.notion.com/v1{path}"
    body = json.dumps(data).encode("utf-8") if data else None
    req = urllib.request.Request(url, data=body, headers=HEADERS, method=method)
    try:
        with urllib.request.urlopen(req) as resp:
            return json.loads(resp.read().decode("utf-8"))
    except urllib.error.HTTPError as e:
        err = e.read().decode("utf-8")
        print(f"HTTP Error {e.code}: {err}")
        raise

def h1(text):
    return {
        "object": "block",
        "type": "heading_1",
        "heading_1": {
            "rich_text": [{"type": "text", "text": {"content": text}}]
        }
    }

def h2(text):
    return {
        "object": "block",
        "type": "heading_2",
        "heading_2": {
            "rich_text": [{"type": "text", "text": {"content": text}}]
        }
    }

def h3(text):
    return {
        "object": "block",
        "type": "heading_3",
        "heading_3": {
            "rich_text": [{"type": "text", "text": {"content": text}}]
        }
    }

def para(text):
    return {
        "object": "block",
        "type": "paragraph",
        "paragraph": {
            "rich_text": [{"type": "text", "text": {"content": text}}]
        }
    }

def para_rich(parts):
    """parts = list of (text, bold, italic, code)"""
    rich = []
    for p in parts:
        if isinstance(p, str):
            rich.append({"type": "text", "text": {"content": p}})
        else:
            text, bold, italic, code = p
            rich.append({
                "type": "text",
                "text": {"content": text},
                "annotations": {"bold": bold, "italic": italic, "code": code, "strikethrough": False, "underline": False, "color": "default"}
            })
    return {
        "object": "block",
        "type": "paragraph",
        "paragraph": {"rich_text": rich}
    }

def bullet(text, bold_prefix=None):
    if bold_prefix:
        return {
            "object": "block",
            "type": "bulleted_list_item",
            "bulleted_list_item": {
                "rich_text": [
                    {"type": "text", "text": {"content": bold_prefix + ": "}, "annotations": {"bold": True, "italic": False, "code": False, "strikethrough": False, "underline": False, "color": "default"}},
                    {"type": "text", "text": {"content": text}}
                ]
            }
        }
    return {
        "object": "block",
        "type": "bulleted_list_item",
        "bulleted_list_item": {
            "rich_text": [{"type": "text", "text": {"content": text}}]
        }
    }

def callout(text, emoji="💡"):
    return {
        "object": "block",
        "type": "callout",
        "callout": {
            "rich_text": [{"type": "text", "text": {"content": text}}],
            "icon": {"type": "emoji", "emoji": emoji}
        }
    }

def divider():
    return {"object": "block", "type": "divider", "divider": {}}

def code_block(text, language="plain text"):
    return {
        "object": "block",
        "type": "code",
        "code": {
            "rich_text": [{"type": "text", "text": {"content": text}}],
            "language": language
        }
    }

def numbered(text, bold_prefix=None):
    if bold_prefix:
        return {
            "object": "block",
            "type": "numbered_list_item",
            "numbered_list_item": {
                "rich_text": [
                    {"type": "text", "text": {"content": bold_prefix + ": "}, "annotations": {"bold": True, "italic": False, "code": False, "strikethrough": False, "underline": False, "color": "default"}},
                    {"type": "text", "text": {"content": text}}
                ]
            }
        }
    return {
        "object": "block",
        "type": "numbered_list_item",
        "numbered_list_item": {
            "rich_text": [{"type": "text", "text": {"content": text}}]
        }
    }

# ──────────────────────────────────────────────────────────────────────────────
# Build the page blocks
# ──────────────────────────────────────────────────────────────────────────────

blocks = [
    callout("pekko-agent는 Rust로 구현된 EHS(환경·보건·안전) 특화 멀티 에이전트 AI 프레임워크입니다. Claude AI API를 기반으로 하며, Apache Pekko에서 영감을 받은 Actor 모델 위에 구축되어 있습니다.", "🤖"),
    divider(),

    # ── 1. 개요
    h1("1. 시스템 개요 (System Overview)"),
    para("pekko-agent는 복잡한 EHS 업무를 자율적으로 처리하는 AI 에이전트 시스템입니다. 자연어 쿼리를 받아 필요한 도구를 선택하고, 결과를 분석하여 전문가 수준의 응답을 생성합니다."),
    para(""),
    h3("핵심 동작 원리 — ReAct 루프"),
    code_block(
        "사용자 질문 입력\n"
        "    ↓\n"
        "[1] Reason  →  \"이 질문에 어떤 도구가 필요한가?\" 추론\n"
        "    ↓\n"
        "[2] Act     →  도구 실행 (permit_search / compliance_check / ...)\n"
        "    ↓\n"
        "[3] Observe →  도구 실행 결과 수집\n"
        "    ↓\n"
        "[4] Respond →  Claude API로 전문가 응답 생성\n"
        "    ↓\n"
        "최종 응답 반환",
        "plain text"
    ),
    divider(),

    # ── 2. 전문 에이전트
    h1("2. 전문 EHS 에이전트 (Specialized Agents)"),
    para("현재 세 가지 전문 에이전트가 구현되어 있으며, 각각 독립 마이크로서비스로 배포됩니다."),

    # 2.1 Permit
    h2("2-1. 허가 관리 에이전트 (Permit Agent)"),
    callout("환경 허가(Permit) 발급·조회·갱신에 특화된 에이전트입니다.", "📋"),
    bullet("기존 허가 검색", "permit_search"),
    bullet("규제 준수 여부 확인 (EPA-40-CFR, OSHA-29-CFR, State-Environmental)", "compliance_check"),
    bullet("허가 신청 문서 자동 생성 (업종별 템플릿 지원: 제조, 화학, 제약, Oil & Gas)", "document_generate"),
    bullet("승인 워크플로우 요청 및 추적", "approval_request"),
    para(""),
    para("지원 상태 흐름: Idle → AnalyzingRequest → CheckingRegulations → GeneratingDocument → ReviewingChecklist → AwaitingApproval → Completed"),

    # 2.2 Inspection
    h2("2-2. 안전 점검 에이전트 (Inspection Agent)"),
    callout("시설 안전 점검 스케줄링, 위험 평가, 보고서 작성을 자동화합니다.", "🔍"),
    bullet("안전 점검 일정 예약 및 담당자 배정", "inspection_schedule"),
    bullet("현장 위험 요소 평가 및 위험 등급 분류", "risk_assessment"),
    bullet("점검 결과(Findings) 기록: 카테고리·심각도·위치 포함", "findings_record"),
    bullet("시정 조치 요청 및 마감일 관리", "corrective_action"),
    bullet("검사 보고서 자동 생성", "report_generate"),
    para(""),
    para("지원 상태 흐름: Idle → PreparingInspection → SchedulingInspection → ConductingInspection → AssessingRisk → GeneratingReport → AwaitingCorrectiveAction → Completed"),

    # 2.3 Compliance
    h2("2-3. 규정 준수 에이전트 (Compliance Agent)"),
    callout("규제 요건 식별부터 Gap 분석, 시정 계획 수립까지 전 과정을 지원합니다.", "⚖️"),
    bullet("적용 가능한 규제 요건 자동 식별", "identify_requirements"),
    bullet("시설 규정 준수 현황 감사", "compliance_audit"),
    bullet("준수 격차(Gap) 분석 — 심각도별 분류", "gap_analysis"),
    bullet("시정 계획(Remediation Plan) 수립 및 담당자·기한 할당", "remediation_plan"),
    bullet("준수율 모니터링 (conformance %) 및 리포트 생성", "compliance_monitoring"),
    para(""),
    para("지원 상태 흐름: Idle → IdentifyingRequirements → CheckingCompliance → AnalyzingGaps → DevelopingRemediationPlan → MonitoringComplianceStatus → GeneratingComplianceReport"),
    divider(),

    # ── 3. 기술 아키텍처
    h1("3. 기술 아키텍처 (Technical Architecture)"),

    h2("3-1. Actor 모델 (rust-pekko 기반)"),
    para("Apache Pekko에서 영감을 받은 Rust Actor 프레임워크로, 각 에이전트는 독립적인 Actor로 동작합니다."),
    bullet("pekko_actor::Actor 트레이트 — 메시지 수신 및 처리 (receive() 메서드)"),
    bullet("pekko_persistence::PersistentActor 트레이트 — 이벤트 소싱 기반 상태 영속성"),
    bullet("AgentMessage 열거형 — Query | Execute | Respond 세 가지 메시지 유형"),
    bullet("Journal Events — 도메인별 이벤트 로그 (PermitJournalEvent, InspectionJournalEvent, ComplianceJournalEvent)"),
    bullet("Snapshots — 빠른 복구를 위한 전체 상태 스냅샷"),

    h2("3-2. 핵심 크레이트 구성"),
    code_block(
        "pekko-agent-core       — AgentActor 트레이트, 메시지 타입, FSM 상태\n"
        "pekko-agent-llm        — Claude API 클라이언트, LLM 게이트웨이, 서킷 브레이커\n"
        "pekko-agent-tools      — 도구 레지스트리, MCP 호환 도구 정의\n"
        "pekko-agent-memory     — 단기(Redis) / 장기(Qdrant) / 에피소딕(PostgreSQL) 메모리\n"
        "pekko-agent-orchestrator — 멀티 에이전트 워크플로우, Saga 패턴\n"
        "pekko-agent-events     — 이벤트 발행/구독 버스\n"
        "pekko-agent-security   — RBAC, 멀티 테넌트 격리",
        "plain text"
    ),

    h2("3-3. 내결함성 (Fault Tolerance)"),
    bullet("Circuit Breaker — LLM API 장애 시 자동 차단 및 복구"),
    bullet("이벤트 소싱 — 에이전트 충돌 후 완전한 상태 복원"),
    bullet("재시도 로직 — HTTP 오류에 대한 지수 백오프"),
    bullet("타임아웃 관리 — 도구별 개별 타임아웃 설정 (5초~15초)"),

    h2("3-4. 보안"),
    bullet("역할 기반 접근 제어 (RBAC) — ehs.permit.read / write / approve 등 세분화"),
    bullet("멀티 테넌트 격리 — tenant_id 기반 데이터 분리"),
    bullet("인증 컨텍스트 — 모든 쿼리에 user_id, tenant_id, roles 포함"),
    divider(),

    # ── 4. 인프라
    h1("4. 인프라 구성 (Infrastructure)"),
    para("Docker Compose 기반으로 로컬 및 프로덕션 환경 모두 지원합니다."),
    bullet("Redis — 대화 이력(단기 메모리) 저장"),
    bullet("Qdrant — 벡터 임베딩 기반 RAG (검색 증강 생성)"),
    bullet("PostgreSQL — 의사결정 이력(에피소딕 메모리) 저장"),
    bullet("gRPC — 에이전트 간 고성능 통신"),
    bullet("HTTP API Gateway — 외부 클라이언트 인터페이스"),
    divider(),

    # ── 5. 실제 활용 예시
    h1("5. 실제 활용 사례 (Use Cases)"),

    h2("예시 쿼리 및 에이전트 반응"),

    h3("🏭 제조 시설 허가 검색"),
    code_block(
        "입력: \"Search for existing permits at manufacturing facility FAC-001\"\n"
        "  → permit_search 도구 실행\n"
        "  → 활성 허가 3건 조회 (Air Discharge, Water Treatment, Hazmat Storage)\n"
        "  → 만료 예정 허가 경고 포함 응답 생성",
        "plain text"
    ),

    h3("⚗️ 화학물질 취급 Gap 분석"),
    code_block(
        "입력: \"Perform gap analysis on our chemical handling at FAC-002\"\n"
        "  → compliance_check 도구로 OSHA 29 CFR 규정 확인\n"
        "  → gap_analysis 수행: Critical 2건, Major 3건, Minor 5건 발견\n"
        "  → 우선순위별 시정 권고사항 포함 보고서 생성",
        "plain text"
    ),

    h3("🔍 위험물질 위험 평가"),
    code_block(
        "입력: \"Conduct a hazardous materials risk assessment at FAC-002\"\n"
        "  → risk_assessment 도구 실행\n"
        "  → 위험 등급: HIGH (위험 요소 7건 식별)\n"
        "  → 즉각 시정 필요 항목 및 30일 내 조치 항목 분류",
        "plain text"
    ),

    h3("📊 시설 전반 준수 현황 모니터링"),
    code_block(
        "입력: \"Develop a remediation plan for identified compliance gaps at FAC-003\"\n"
        "  → 12개 시정 조치 항목 생성\n"
        "  → 담당자 배정 + 기한 설정 자동화\n"
        "  → 전체 준수율 87.3% → 목표 99% 달성 계획 수립",
        "plain text"
    ),
    divider(),

    # ── 6. 멀티 에이전트 워크플로우
    h1("6. 멀티 에이전트 워크플로우 (Orchestration)"),
    para("Orchestrator가 여러 에이전트를 조율하여 복잡한 EHS 업무를 자동화합니다."),
    code_block(
        "WorkflowStep 1: InspectionAgent  → 시설 점검 수행\n"
        "WorkflowStep 2: ComplianceAgent  → 점검 결과 기반 규정 확인 (Step1 완료 후)\n"
        "WorkflowStep 3: PermitAgent      → 필요 허가 갱신 처리 (Step2 완료 후)\n"
        "\n"
        "이벤트 발행:\n"
        "  inspection.completed → compliance.check_triggered → permit.renewal_initiated",
        "plain text"
    ),
    bullet("Saga 패턴 지원 — 분산 트랜잭션 실패 시 보상 액션 자동 실행"),
    bullet("이벤트 버스 — 에이전트 간 비동기 이벤트 스트리밍"),
    bullet("병렬 실행 지원 — 독립적인 워크플로우 단계 동시 처리"),
    divider(),

    # ── 7. 확장 가능성
    h1("7. 확장 가능성 (Extensibility)"),
    para("현재 EHS 도메인에 특화되어 있지만, 아키텍처는 범용 멀티 에이전트 시스템으로 확장 가능합니다."),
    numbered("새 에이전트 추가: AgentActor + Actor + PersistentActor 트레이트 구현"),
    numbered("새 도구 등록: ToolDefinition으로 JSON Schema 기반 도구 정의"),
    numbered("MCP 호환: Model Context Protocol 표준 준수로 외부 도구 연동"),
    numbered("RAG 확장: Qdrant 벡터 스토어로 도메인 지식 베이스 구축"),
    numbered("이벤트 확장: EventPublisher로 실시간 알림·모니터링 연동"),
    divider(),

    # ── 8. 기술 스택
    h1("8. 기술 스택 요약"),
    code_block(
        "언어:        Rust (async/await, tokio)\n"
        "AI:          Anthropic Claude API (claude-3-5-sonnet)\n"
        "Actor:       rust-pekko (pekko-actor, pekko-persistence)\n"
        "메모리:       Redis (단기) + Qdrant (벡터) + PostgreSQL (에피소딕)\n"
        "통신:        gRPC (tonic) + HTTP REST\n"
        "직렬화:      serde_json, protobuf\n"
        "배포:        Docker Compose (로컬 + 프로덕션)\n"
        "보안:        RBAC + 멀티 테넌트 격리\n"
        "내결함성:    Circuit Breaker + 이벤트 소싱 + 재시도",
        "plain text"
    ),
    divider(),
    callout("이 시스템은 EHS 전문가의 업무를 대체하는 것이 아니라, 반복적이고 복잡한 규정 준수 작업을 자동화하여 전문가가 고부가가치 업무에 집중할 수 있도록 지원합니다.", "🎯"),
]

# ──────────────────────────────────────────────────────────────────────────────
# Create Notion page
# ──────────────────────────────────────────────────────────────────────────────

# Notion has a limit of 100 blocks per API call
CHUNK_SIZE = 99

def create_page_with_blocks(title, parent_id, all_blocks):
    # Create page with first chunk
    first_chunk = all_blocks[:CHUNK_SIZE]
    data = {
        "parent": {"page_id": parent_id},
        "properties": {
            "title": {
                "title": [{"type": "text", "text": {"content": title}}]
            }
        },
        "children": first_chunk
    }
    result = api_call("POST", "/pages", data)
    page_id = result["id"]
    print(f"✅ Page created: {result['url']}")

    # Append remaining blocks in chunks
    remaining = all_blocks[CHUNK_SIZE:]
    offset = 0
    while remaining:
        chunk = remaining[:CHUNK_SIZE]
        remaining = remaining[CHUNK_SIZE:]
        api_call("PATCH", f"/blocks/{page_id}/children", {"children": chunk})
        offset += len(chunk)
        print(f"  Appended {offset} more blocks...")

    return result

print("Creating Notion capability page...")
result = create_page_with_blocks(
    title="pekko-agent로 할 수 있는 것들 (Agentic AI 기능 정리)",
    parent_id=PARENT_PAGE_ID,
    all_blocks=blocks
)
print(f"\n🎉 Done! Page URL: {result['url']}")
