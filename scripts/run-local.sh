#!/bin/bash
# pekko-agent 로컬 실행 스크립트
# PostgreSQL: docker compose -f docker-compose-local.yml up -d
# Rust 서비스: cargo run --bin <service>
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

# ── 색상 ──
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
RED='\033[0;31m'
NC='\033[0m'

usage() {
    echo "Usage: $0 [command]"
    echo ""
    echo "Commands:"
    echo "  infra-up    PostgreSQL 컨테이너 시작 (docker compose -f docker-compose-local.yml)"
    echo "  infra-down  PostgreSQL 컨테이너 중지"
    echo "  start       모든 Rust 서비스 빌드 및 시작 (인프라는 이미 실행 중이어야 함)"
    echo "  start-one   단일 서비스 시작 (예: $0 start-one api-gateway)"
    echo "  stop        모든 Rust 서비스 중지"
    echo "  status      서비스 상태 확인"
    echo "  build       릴리스 빌드만 수행"
    echo "  logs        서비스 로그 보기 (예: $0 logs api-gateway)"
    echo ""
    echo "빠른 시작:"
    echo "  1. cp .env.example .env && vi .env   # API 키 설정"
    echo "  2. $0 infra-up                       # PostgreSQL 시작"
    echo "  3. $0 start                          # 서비스 시작"
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
        echo -e "  ${RED}❌${NC} .env 파일이 없습니다."
        echo "     cp .env.example .env && vi .env"
        exit 1
    fi
}

# ── 인프라 시작 (PostgreSQL) ──
cmd_infra_up() {
    echo -e "${CYAN}🐘 PostgreSQL 컨테이너 시작${NC}"
    cd "$PROJECT_DIR"
    docker compose -f docker-compose-local.yml up -d
    echo ""
    echo -e "${GREEN}✅ 인프라 시작 완료${NC}"
    echo "   DATABASE_URL=postgres://pekko:pekko_secret@localhost:5432/pekko_agent"
}

# ── 인프라 중지 ──
cmd_infra_down() {
    echo -e "${CYAN}🛑 PostgreSQL 컨테이너 중지${NC}"
    cd "$PROJECT_DIR"
    docker compose -f docker-compose-local.yml down
    echo -e "${GREEN}✅ 완료${NC}"
}

# ── PostgreSQL 연결 확인 ──
check_postgres() {
    local db_url="${DATABASE_URL:-postgres://pekko:pekko_secret@localhost:5432/pekko_agent}"
    if command -v pg_isready &>/dev/null; then
        if pg_isready -d "$db_url" -q; then
            echo -e "  ${GREEN}✅${NC} PostgreSQL 연결 확인"
            return 0
        fi
    elif command -v psql &>/dev/null; then
        if psql "$db_url" -c "SELECT 1" -q &>/dev/null; then
            echo -e "  ${GREEN}✅${NC} PostgreSQL 연결 확인"
            return 0
        fi
    fi
    echo -e "  ${YELLOW}⚠️${NC}  PostgreSQL에 연결할 수 없습니다."
    echo "     먼저 '$0 infra-up' 을 실행하세요."
    return 1
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
    echo -e "${CYAN}🚀 pekko-agent 서비스 시작${NC}"
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

    load_env

    echo ""
    echo "🗄️  PostgreSQL 연결 확인..."
    check_postgres || exit 1

    echo ""
    cmd_build

    echo ""
    echo "📡 서비스 시작..."
    local SERVICES=("api-gateway" "ehs-permit-agent" "ehs-inspection-agent" "ehs-compliance-agent")

    for svc in "${SERVICES[@]}"; do
        local bin="$PROJECT_DIR/target/release/$svc"
        if [ ! -f "$bin" ]; then
            echo -e "  ${YELLOW}⚠️${NC}  $svc 바이너리 없음"
            continue
        fi

        echo "  📌 $svc 시작 중..."
        nohup "$bin" > "/tmp/pekko-${svc}.log" 2>&1 &
        local pid=$!
        echo "$pid" > "/tmp/pekko-${svc}.pid"
        echo -e "  ${GREEN}✅${NC} $svc (PID: $pid)"
    done

    echo ""
    echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
    echo -e "${GREEN}✅ 서비스 시작 완료!${NC}"
    echo ""
    echo "📍 엔드포인트:"
    echo "   API Gateway : http://localhost:${API_GATEWAY_PORT:-8080}"
    echo "   Health Check: curl http://localhost:${API_GATEWAY_PORT:-8080}/api/health"
    echo "   JWT 발급    : curl -X POST http://localhost:${API_GATEWAY_PORT:-8080}/api/auth/token"
    echo ""
    echo "📍 로그 확인:  $0 logs api-gateway"
    echo "📍 상태 확인:  $0 status"
    echo "📍 중지:       $0 stop"
}

# ── 단일 서비스 시작 ──
cmd_start_one() {
    local svc="${1:?서비스 이름을 지정하세요 (예: api-gateway)}"
    echo -e "${CYAN}🚀 $svc 시작${NC}"

    load_env
    check_postgres || exit 1

    cd "$PROJECT_DIR"
    echo "  📌 빌드 중..."
    cargo build --release -p "$svc" 2>&1

    local bin="$PROJECT_DIR/target/release/$svc"
    nohup "$bin" > "/tmp/pekko-${svc}.log" 2>&1 &
    local pid=$!
    echo "$pid" > "/tmp/pekko-${svc}.pid"
    echo -e "  ${GREEN}✅${NC} $svc 시작 (PID: $pid)"
    echo "  📍 로그: tail -f /tmp/pekko-${svc}.log"
}

# ── 로그 보기 ──
cmd_logs() {
    local svc="${1:-api-gateway}"
    local logfile="/tmp/pekko-${svc}.log"
    if [ -f "$logfile" ]; then
        tail -f "$logfile"
    else
        echo -e "${YELLOW}로그 파일 없음:${NC} $logfile"
    fi
}

# ── 중지 ──
cmd_stop() {
    echo -e "${CYAN}🛑 pekko-agent 서비스 중지${NC}"
    local SERVICES=("api-gateway" "ehs-permit-agent" "ehs-inspection-agent" "ehs-compliance-agent")
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
    local db_url="${DATABASE_URL:-postgres://pekko:pekko_secret@localhost:5432/pekko_agent}"
    if command -v pg_isready &>/dev/null && pg_isready -d "$db_url" -q 2>/dev/null; then
        echo -e "  ${GREEN}●${NC} PostgreSQL (${db_url%%@*}@...)"
    else
        echo -e "  ○ PostgreSQL (연결 안됨 — '$0 infra-up' 실행 필요)"
    fi
}

# ── 메인 ──
case "${1:-help}" in
    infra-up)   cmd_infra_up ;;
    infra-down) cmd_infra_down ;;
    start)      cmd_start ;;
    start-one)  cmd_start_one "${2:-}" ;;
    stop)       cmd_stop ;;
    status)     cmd_status ;;
    build)      cmd_build ;;
    logs)       cmd_logs "${2:-api-gateway}" ;;
    *)          usage ;;
esac
