# EHS Agent Services - Implementation Guide

## Quick Reference

### File Sizes
- **ehs-permit-agent/src/permit_agent.rs**: ~18.6 KB (comprehensive permit lifecycle)
- **ehs-inspection-agent/src/inspection_agent.rs**: ~20.7 KB (detailed inspection tracking)
- **ehs-compliance-agent/src/compliance_agent.rs**: ~24.9 KB (extensive compliance rules)

---

## Agent-Specific Implementations

### 1. Permit Agent (permit_agent.rs)

**Key Classes:**
```rust
pub struct PermitAgentActor {
    agent_id: String,
    state: AgentState,
    permit_state: PermitAgentState,          // Domain-specific state
    execution_context: ExecutionContext,     // Track facility & permits
}

pub enum PermitAgentState {
    Idle,
    AnalyzingRequest { request_id, industry },
    CheckingRegulations { regulations },
    GeneratingDocument { doc_type, facility_id },
    ReviewingChecklist { items },
    AwaitingApproval { approver, permit_id },
    Completed { permit_id, issued_date },
}
```

**Intelligence Features:**
- Extracts facility IDs (FAC-XXX) and industry types from queries
- Routes requests based on keywords: "search", "compliance", "generate", "approve"
- Maintains active permit inventory
- Generates realistic permit documents with IDs and expiry dates
- Tracks approval workflows

**Tool Implementations:**
```
permit_search     → Returns 3 sample permits (active, expired)
document_generate → Creates permit application with 12 pages
compliance_check  → 92% compliance score with violation details
approval_request  → Routes to Facility Manager for review
```

---

### 2. Inspection Agent (inspection_agent.rs)

**Key Classes:**
```rust
pub struct InspectionAgentActor {
    agent_id: String,
    state: AgentState,
    inspection_state: InspectionAgentState,   // Domain-specific state
    execution_context: ExecutionContext,      // Track findings & risk
}

pub enum InspectionAgentState {
    Idle,
    PreparingInspection { facility_id, inspection_type },
    SchedulingInspection { proposed_dates, inspector_ids },
    ConductingInspection { inspection_id, findings },
    AssessingRisk { risk_level, hazard_count },
    GeneratingReport { report_id },
    AwaitingCorrectiveAction { inspection_id, deadline },
    Completed { inspection_id, report_id },
}
```

**Intelligence Features:**
- Detects inspection types: Routine, Safety, Hazmat, Environmental, Follow-up
- Classifies findings by severity: Critical, Major, Minor
- Assigns multiple inspectors with specializations
- Calculates risk scores (0-100)
- Sets realistic inspection timelines (multi-day durations)

**Tool Implementations:**
```
inspection_create      → Creates record with priority and type
inspection_schedule    → Assigns 2 inspectors with specializations
risk_assessment        → Medium risk, 2 high-risk areas, 7 total hazards
findings_document      → 12 findings (1 critical, 3 major, 8 minor)
```

---

### 3. Compliance Agent (compliance_agent.rs)

**Key Classes:**
```rust
pub struct ComplianceAgentActor {
    agent_id: String,
    state: AgentState,
    compliance_state: ComplianceAgentState,   // Domain-specific state
    execution_context: ExecutionContext,      // Track gaps & actions
}

pub enum ComplianceAgentState {
    Idle,
    IdentifyingRequirements { facility_id, regulations },
    CheckingCompliance { audit_id, check_count },
    AnalyzingGaps { gap_count, severity_breakdown },
    DevelopingRemediationPlan { plan_id, action_items },
    MonitoringComplianceStatus { facility_id, conformance_percentage },
    GeneratingComplianceReport { report_id },
}
```

**Intelligence Features:**
- Auto-detects applicable regulations by facility type:
  - "manufacturing" → Air Quality + Safety standards
  - "chemical" → EPCRA + Process Safety
  - "oil/gas" → Multiple environmental standards
- Severity-based gap analysis: Critical, Major, Minor
- Generates 90-day remediation timelines
- Calculates conformance percentages (0-100%)
- Tracks action item ownership and deadlines

**Tool Implementations:**
```
compliance_check      → 88% conformance (18 passed, 3 failed, 1 pending)
regulation_lookup     → Returns title, effective date, penalties
gap_analysis          → 7 gaps total (1 critical, 3 major, 3 minor)
remediation_plan      → 8 actions, $150K budget, 90-day timeline
compliance_report     → Score 85, trend improving, next audit June 2026
```

---

## Query Processing Patterns

### Permit Agent Example
```
User: "Search for active environmental permits for facility FAC-001"

1. reason() → Extracts FAC-001, detects "search" keyword
2. act()    → Calls permit_search tool
3. respond()→ Lists 3 permits with status/expiry, suggests renewal

Output: "Found 3 permits: 2 Active (expires 2025-2026), 1 Expired (2023)"
```

### Inspection Agent Example
```
User: "Schedule a routine safety inspection for facility FAC-001"

1. reason() → Extracts FAC-001, detects "routine" + "safety"
2. act()    → Calls inspection_create + risk_assessment
3. respond()→ Confirms scheduled 3-day inspection with 2 inspectors

Output: "Inspection scheduled 2026-03-15 to 2026-03-17 with INS-001, INS-002"
```

### Compliance Agent Example
```
User: "Check regulatory compliance for manufacturing facility FAC-001"

1. reason() → Detects "manufacturing" industry type
2. act()    → Identifies EPA/OSHA/State regulations, calls compliance_check
3. respond()→ Returns 88% conformance, identifies 3 failed checks

Output: "Compliance Score: 88% | 7 gaps identified (1 critical, 3 major)"
```

---

## Tool Execution Characteristics

All agents follow this pattern for tool execution:

```rust
async fn act(&mut self, action: &AgentAction) -> Result<Vec<Observation>, AgentError> {
    match action {
        AgentAction::UseTool(calls) => {
            let mut observations = Vec::new();
            
            for call in calls {
                let result = match call.name.as_str() {
                    "tool_name" => {
                        serde_json::json!({
                            "status": "success/completed/created",
                            // Domain-specific results
                        })
                    }
                    _ => error_response
                };
                
                observations.push(Observation {
                    tool_call_id: call.id.clone(),
                    tool_name: call.name.clone(),
                    result,
                    is_error: false,
                    duration_ms: 150-250,  // Realistic simulated delays
                });
            }
            
            Ok(observations)
        }
    }
}
```

---

## System Prompts

### Permit Agent
> "You are an EHS Permit Agent specializing in environmental permit management...
>  Help users search for permits, verify compliance, generate permit documents,
>  and manage the permit approval workflow. Always check regulatory requirements..."

### Inspection Agent
> "You are an EHS Inspection Agent specialized in conducting environmental,
>  health, and safety inspections... prioritize worker safety, regulatory compliance,
>  and corrective action tracking..."

### Compliance Agent
> "You are an EHS Compliance Agent specialized in environmental, health, and safety
>  regulatory compliance... identify applicable regulations, check facility compliance status,
>  analyze compliance gaps, develop remediation plans..."

---

## Max Iterations
- **Permit Agent**: 8 iterations (straightforward workflows)
- **Inspection Agent**: 10 iterations (multi-phase inspections)
- **Compliance Agent**: 10 iterations (complex regulatory analysis)

---

## Token Usage Estimates (per response)

| Agent | Input Tokens | Output Tokens | Total |
|-------|-------------|---------------|-------|
| Permit | 250 | 180 | 430 |
| Inspection | 300 | 220 | 520 |
| Compliance | 350 | 250 | 600 |

---

## Execution Context Tracking

Each agent maintains an `ExecutionContext` to avoid redundant queries:

### Permit Agent Context
```rust
struct ExecutionContext {
    current_facility: Option<String>,      // FAC-001
    industry_type: Option<String>,         // Manufacturing
    active_permits: Vec<String>,           // PERMIT-2024-001, ...
}
```

### Inspection Agent Context
```rust
struct ExecutionContext {
    facility_id: Option<String>,
    inspection_type: Option<String>,       // Routine, Safety, etc.
    current_findings: Vec<Finding>,        // Accumulated findings
    inspector_assignment: HashMap<String, String>,  // INS-001 → Safety
}
```

### Compliance Agent Context
```rust
struct ExecutionContext {
    facility_id: Option<String>,
    applicable_regulations: Vec<String>,   // EPA, OSHA, State, ...
    identified_gaps: Vec<ComplianceGap>,   // Compliance deficiencies
    remediation_actions: Vec<RemediationAction>,  // Corrective actions
}
```

---

## Testing Recommendations

### Unit Testing Patterns
1. **State Transitions**: Verify correct state changes in each agent
2. **Tool Selection**: Test reasoning logic selects correct tools
3. **Data Extraction**: Validate facility/industry parsing from queries
4. **Mock Data**: Ensure realistic result generation

### Integration Testing
1. **Multi-tool workflows**: Verify action chains (e.g., create → schedule)
2. **State persistence**: Confirm execution context carries across calls
3. **Error handling**: Test graceful degradation for invalid inputs

### Example Test for Permit Agent
```rust
#[tokio::test]
async fn test_permit_search_workflow() {
    let mut agent = PermitAgentActor::new("test-permit");
    let query = UserQuery {
        content: "Search permits for FAC-001".to_string(),
        // ... other fields
    };
    
    let action = agent.reason(&query).await.unwrap();
    assert!(matches!(action, AgentAction::UseTool(_)));
    
    let observations = agent.act(&action).await.unwrap();
    assert!(!observations.is_empty());
    
    let response = agent.respond(&observations).await.unwrap();
    assert!(response.content.contains("permits_found"));
}
```

---

## Compilation & Deployment

### Build Individual Services
```bash
cd services/ehs-permit-agent && cargo build --release
cd services/ehs-inspection-agent && cargo build --release
cd services/ehs-compliance-agent && cargo build --release
```

### Run Demo
```bash
cargo run --bin ehs-permit-agent
cargo run --bin ehs-inspection-agent
cargo run --bin ehs-compliance-agent
```

### Production Deployment
Each agent runs on dedicated port with gRPC endpoint:
- **Permit Agent**: Port 50055
- **Inspection Agent**: Port 50056
- **Compliance Agent**: Port 50057

---

## Key Differentiators

The three agents demonstrate how the Pekko Agent framework scales:

| Dimension | Permit | Inspection | Compliance |
|-----------|--------|-----------|-----------|
| **Domain** | Documents | Audits | Regulations |
| **Workflow** | Linear | Iterative | Analytical |
| **Data Volume** | Moderate | High | Very High |
| **State Mgmt** | Simple | Complex | Very Complex |
| **Tool Types** | CRUD | Analysis | Decision-making |

Each maintains domain-specific intelligence while sharing core architecture.
