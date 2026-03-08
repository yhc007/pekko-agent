#!/bin/bash
# pekko-agent 로컬 실행 스크립트
# 모든 인프라가 임베디드이므로 외부 서비스는 Qdrant만 필요
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"
DATA_DIR="${COREDB_DATA_DIR:-$PROJECT_DIR/data/coredb}"

# ── 색상 ──
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m'

usage() {
    echo "Usage: $0 [command]"
    echo ""
    echo "Commands:"
    echo "  setup      Qdrant 설치 확인 및 데이터 디렉토리 준비"
    echo "  start      모든 서비스 빌드 및 시작"
    echo "  start-one  단일 서비스 시작 (예: $0 start-one api-gateway)"
    echo "  stop       모든 서비스 중지"
    echo "  status     서비스 상태 확인"
    echo "  build      릴리스 빌드만 수행"
    echo ""
    echo "Architecture:"
    echo "  coreDB       → embedded (in-process, 자동)"
    echo "  EventBus     → pekko-event-bus (in-process, 자동)"
    echo "  Qdrant       → 로컬 바이너리 또는 원격 서버"
    exit 0
}

# ── .env 로드 ──
load_env() {
    if [ -f "$PROJECT_DIR/.env" ]; then
        set -a
        source "$PROJECT_DIR/.env"
        set +a
        echo -e "  ${GREEN}✅${NC} .env 로드 완료"
    else
        echo -e "  ${YELLOW}⚠️${NC}  .env 파일 없음 — .env.example을 복사하세요:"
        echo "     cp .env.example .env && vi .env"
        exit 1
    fi
}

# ── 셋업 ──
cmd_setup() {
    echo -e "${CYAN}🔧 pekko-agent 로컬 환경 셋업${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    # 1. coreDB 데이터 디렉토리
    echo ""
    echo "📦 [1/3] coreDB 데이터 디렉토리 준비..."
    mkdir -p "$DATA_DIR"
    echo -e "  ${GREEN}✅${NC} $DATA_DIR"

    # 2. Qdrant
    echo ""
    echo "📦 [2/3] Qdrant 확인..."
    if command -v qdrant &>/dev/null; then
        echo -e "  ${GREEN}✅${NC} Qdrant 설치됨: $(qdrant --version 2>/dev/null || echo 'installed')"
    else
        echo -e "  ${YELLOW}⚠️${NC}  Qdrant 미설치"
        echo "     설치: brew install qdrant"
        echo "     또는 원격 서버 사용: QDRANT_URL=http://your-server:6333"
    fi

    # 3. .env 파일
    echo ""
    echo "📦 [3/3] 환경 설정 파일..."
    if [ ! -f "$PROJECT_DIR/.env" ]; then
        cp "$PROJECT_DIR/.env.example" "$PROJECT_DIR/.env"
        echo -e "  ${GREEN}✅${NC} .env 생성 (.env.example 복사)"
        echo -e "  ${YELLOW}⚠️${NC}  CLAUDE_API_KEY를 실제 키로 변경하세요!"
    else
        echo -e "  ${GREEN}✅${NC} .env 이미 존재"
    fi

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo -e "${GREEN}✅ 셋업 완료!${NC} 다음 단계: $0 start"
}

# ── Qdrant 시작 ──
ensure_qdrant() {
    local qdrant_url="${QDRANT_URL:-http://localhost:6333}"

    if curl -sf "${qdrant_url}/healthz" > /dev/null 2>&1; then
        echo -e "  ${GREEN}✅${NC} Qdrant 실행 중 (${qdrant_url})"
        return 0
    fi

    # 로컬에 설치되어 있으면 자동 시작
    if command -v qdrant &>/dev/null && [[ "$qdrant_url" == *"localhost"* ]]; then
        echo "  📌 Qdrant 시작 중..."
        local qdrant_data="${PROJECT_DIR}/data/qdrant"
        mkdir -p "$qdrant_data"
        nohup qdrant --storage-path "$qdrant_data" > /tmp/pekko-qdrant.log 2>&1 &
        echo $! > /tmp/pekko-qdrant.pid
        sleep 3
        if curl -sf "${qdrant_url}/healthz" > /dev/null 2>&1; then
            echo -e "  ${GREEN}✅${NC} Qdrant 시작 완료 (PID: $(cat /tmp/pekko-qdrant.pid))"
        else
            echo -e "  ${YELLOW}⚠️${NC}  Qdrant 시작 실패 — cat /tmp/pekko-qdrant.log 확인"
        fi
    else
        echo -e "  ${YELLOW}⚠️${NC}  Qdrant에 연결할 수 없음: ${qdrant_url}"
        echo "     Qdrant 없이도 실행 가능 (벡터 검색 비활성화)"
    fi
}

# ── 빌드 ──
cmd_build() {
    echo -e "${CYAN}🔨 릴리스 빌드 중...${NC}"
    cd "$PROJECT_DIR"
    cargo build --release 2>&1
    echo -e "${GREEN}✅ 빌드 완료${NC}"
}

# ── 전체 시작 ──
cmd_start() {
    echo -e "${CYAN}🚀 pekko-agent 로컬 시작${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    load_env
    mkdir -p "$DATA_DIR"

    # Qdrant
    echo ""
    echo "📡 외부 서비스 확인..."
    ensure_qdrant

    # 빌드
    echo ""
    cmd_build

    # 서비스 시작
    echo ""
    echo "📡 서비스 시작..."
    local SERVICES=("api-gateway" "ehs-permit-agent" "ehs-inspection-agent" "ehs-compliance-agent")
    local PIDS=()

    for svc in "${SERVICES[@]}"; do
        local bin="$PROJECT_DIR/target/release/$svc"
        if [ ! -f "$bin" ]; then
            echo -e "  ${YELLOW}⚠️${NC}  $svc 바이너리 없음 (빌드 에러 확인)"
            continue
        fi

        echo "  📌 $svc 시작 중..."
        COREDB_DATA_DIR="$DATA_DIR/$svc" \
        nohup "$bin" > "/tmp/pekko-${svc}.log" 2>&1 &
        local pid=$!
        PIDS+=("$pid")
        echo "$pid" > "/tmp/pekko-${svc}.pid"
        echo -e "  ${GREEN}✅${NC} $svc (PID: $pid)"
    done

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo -e "${GREEN}✅ 모든 서비스 시작 완료!${NC}"
    echo ""
    echo "📍 엔드포인트:"
    echo "   API Gateway : http://localhost:${API_GATEWAY_PORT:-8080}"
    echo "   Health Check: curl http://localhost:${API_GATEWAY_PORT:-8080}/api/health"
    echo ""
    echo "📍 로그 확인:"
    echo "   tail -f /tmp/pekko-api-gateway.log"
    echo ""
    echo "📍 중지:"
    echo "   $0 stop"
}

# ── 단일 서비스 시작 ──
cmd_start_one() {
    local svc="${1:?서비스 이름을 지정하세요 (예: api-gateway)}"
    echo -e "${CYAN}🚀 $svc 시작${NC}"

    load_env
    mkdir -p "$DATA_DIR/$svc"

    cd "$PROJECT_DIR"
    echo "  📌 빌드 중..."
    cargo build --release -p "$svc" 2>&1

    local bin="$PROJECT_DIR/target/release/$svc"
    echo "  📌 실행 중..."
    COREDB_DATA_DIR="$DATA_DIR/$svc" \
    nohup "$bin" > "/tmp/pekko-${svc}.log" 2>&1 &
    local pid=$!
    echo "$pid" > "/tmp/pekko-${svc}.pid"
    echo -e "  ${GREEN}✅${NC} $svc (PID: $pid)"
    echo "  📍 로그: tail -f /tmp/pekko-${svc}.log"
}

# ── 중지 ──
cmd_stop() {
    echo -e "${CYAN}🛑 pekko-agent 서비스 중지${NC}"

    local SERVICES=("api-gateway" "ehs-permit-agent" "ehs-inspection-agent" "ehs-compliance-agent" "qdrant")

    for svc in "${SERVICES[@]}"; do
        local pidfile="/tmp/pekko-${svc}.pid"
        if [ -f "$pidfile" ]; then
            local pid=$(cat "$pidfile")
            if kill -0 "$pid" 2>/dev/null; then
                kill "$pid"
                echo -e "  ${GREEN}✅${NC} $svc 중지 (PID: $pid)"
            else
                echo "  ⏭️  $svc 이미 중지됨"
            fi
            rm -f "$pidfile"
        fi
    done

    echo -e "${GREEN}✅ 완료${NC}"
}

# ── 상태 ──
cmd_status() {
    echo -e "${CYAN}📊 pekko-agent 서비스 상태${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    local SERVICES=("api-gateway" "ehs-permit-agent" "ehs-inspection-agent" "ehs-compliance-agent")

    for svc in "${SERVICES[@]}"; do
        local pidfile="/tmp/pekko-${svc}.pid"
        if [ -f "$pidfile" ] && kill -0 "$(cat "$pidfile")" 2>/dev/null; then
            echo -e "  ${GREEN}●${NC} $svc (PID: $(cat "$pidfile"))"
        else
            echo -e "  ○ $svc (중지됨)"
        fi
    done

    echo ""
    local qdrant_url="${QDRANT_URL:-http://localhost:6333}"
    if curl -sf "${qdrant_url}/healthz" > /dev/null 2>&1; then
        echo -e "  ${GREEN}●${NC} Qdrant (${qdrant_url})"
    else
        echo -e "  ○ Qdrant (${qdrant_url} — 연결 안됨)"
    fi
}

# ── 메인 ──
case "${1:-help}" in
    setup)     cmd_setup ;;
    start)     cmd_start ;;
    start-one) cmd_start_one "${2:-}" ;;
    stop)      cmd_stop ;;
    status)    cmd_status ;;
    build)     cmd_build ;;
    *)         usage ;;
esac
