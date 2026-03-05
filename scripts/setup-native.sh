#!/bin/bash
# pekko-agent 로컬 셋업 (Mac Studio)
# PostgreSQL: 기존 설치 사용 (네이티브)
# Redis:      brew (네이티브)
# Qdrant:     brew (네이티브)
# Redpanda:   Docker 컨테이너 1개 (macOS 네이티브 불가)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "🚀 pekko-agent 로컬 환경 셋업 (Mac Studio)"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  PostgreSQL : 네이티브 (기존 설치)"
echo "  Redis      : 네이티브 (brew)"
echo "  Qdrant     : 네이티브 (brew)"
echo "  Redpanda   : Docker 컨테이너"

# ── 1. Homebrew 패키지 설치 ──
echo ""
echo "📦 [1/5] Homebrew 패키지 확인 및 설치..."

install_if_missing() {
    local pkg=$1
    local name=${2:-$1}
    if brew list "$pkg" &>/dev/null; then
        echo "  ✅ $name 이미 설치됨"
    else
        echo "  📌 $name 설치 중..."
        brew install "$pkg"
        echo "  ✅ $name 설치 완료"
    fi
}

install_if_missing redis "Redis"
install_if_missing qdrant "Qdrant"
install_if_missing redpanda-data/tap/redpanda "Redpanda rpk CLI"

# ── 2. PostgreSQL 데이터베이스 준비 ──
echo ""
echo "🐘 [2/5] PostgreSQL 데이터베이스 준비..."

if psql -lqt | cut -d \| -f 1 | grep -qw pekko_agent; then
    echo "  ✅ 'pekko_agent' DB 이미 존재"
else
    echo "  📌 'pekko_agent' DB 생성 중..."
    createdb pekko_agent 2>/dev/null || true
    echo "  ✅ DB 생성 완료"
fi

psql -d pekko_agent -c "
    DO \$\$
    BEGIN
        IF NOT EXISTS (SELECT FROM pg_catalog.pg_roles WHERE rolname = 'pekko') THEN
            CREATE ROLE pekko WITH LOGIN PASSWORD 'pekko_secret';
        END IF;
    END
    \$\$;
    GRANT ALL PRIVILEGES ON DATABASE pekko_agent TO pekko;
" 2>/dev/null || true

echo "  📌 스키마 적용 중..."
psql -d pekko_agent -f "$PROJECT_DIR/infra/init.sql" 2>/dev/null || {
    echo "  ⚠️  스키마 이미 적용됨 (중복 무시)"
}

psql -d pekko_agent -c "
    GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO pekko;
    GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO pekko;
    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES TO pekko;
" 2>/dev/null || true

echo "  ✅ PostgreSQL 준비 완료"

# ── 3. Redis 시작 (네이티브) ──
echo ""
echo "🔴 [3/5] Redis 시작 (네이티브)..."

if redis-cli ping 2>/dev/null | grep -q PONG; then
    echo "  ✅ Redis 이미 실행 중 (localhost:6379)"
else
    echo "  📌 Redis 시작 중..."
    brew services start redis
    sleep 2
    if redis-cli ping 2>/dev/null | grep -q PONG; then
        echo "  ✅ Redis 시작 완료 (localhost:6379)"
    else
        echo "  ❌ Redis 시작 실패 — brew services restart redis"
    fi
fi

# ── 4. Qdrant 시작 (네이티브) ──
echo ""
echo "🔵 [4/5] Qdrant 시작 (네이티브)..."

if curl -sf http://localhost:6333/healthz > /dev/null 2>&1; then
    echo "  ✅ Qdrant 이미 실행 중 (localhost:6333)"
else
    echo "  📌 Qdrant 시작 중..."

    QDRANT_DATA="$HOME/.local/share/qdrant"
    mkdir -p "$QDRANT_DATA"

    nohup qdrant --storage-path "$QDRANT_DATA" > /tmp/qdrant.log 2>&1 &
    echo $! > /tmp/qdrant.pid

    sleep 3
    if curl -sf http://localhost:6333/healthz > /dev/null 2>&1; then
        echo "  ✅ Qdrant 시작 완료 (localhost:6333)"
        echo "  📍 Dashboard: http://localhost:6333/dashboard"
    else
        echo "  ❌ Qdrant 시작 실패 — cat /tmp/qdrant.log 확인"
    fi
fi

# ── 5. Redpanda 시작 (Docker 컨테이너) ──
echo ""
echo "🐼 [5/5] Redpanda 시작 (Docker)..."
echo "  ℹ️  Redpanda 서버는 Linux 전용이라 Docker가 필요합니다."

# Docker 실행 확인
if ! docker info &>/dev/null; then
    echo "  ❌ Docker가 실행되지 않고 있습니다."
    echo "  📌 Docker Desktop을 시작한 후 다시 실행해주세요."
    echo "  📌 또는 Redpanda 없이 in-memory 이벤트로 테스트할 수 있습니다."
    REDPANDA_RUNNING=false
else
    # 기존 Redpanda 컨테이너 확인
    if docker ps --format '{{.Names}}' | grep -q pekko-redpanda; then
        echo "  ✅ Redpanda 컨테이너 이미 실행 중"
        REDPANDA_RUNNING=true
    else
        echo "  📌 Redpanda 컨테이너 시작 중..."
        docker run -d \
            --name pekko-redpanda \
            -p 19092:19092 \
            -p 18082:18082 \
            -p 18081:18081 \
            -p 9644:9644 \
            -v pekko-redpanda-data:/var/lib/redpanda/data \
            redpandadata/redpanda:v24.1.1 \
            redpanda start \
                --smp 1 \
                --memory 512M \
                --overprovisioned \
                --node-id 0 \
                --kafka-addr internal://0.0.0.0:9092,external://0.0.0.0:19092 \
                --advertise-kafka-addr internal://redpanda:9092,external://localhost:19092 \
                --pandaproxy-addr internal://0.0.0.0:8082,external://0.0.0.0:18082 \
                --advertise-pandaproxy-addr internal://redpanda:8082,external://localhost:18082 \
                --schema-registry-addr internal://0.0.0.0:8081,external://0.0.0.0:18081 \
                --advertise-schema-registry-addr internal://redpanda:8081,external://localhost:18081

        sleep 5
        REDPANDA_RUNNING=true
        echo "  ✅ Redpanda 시작 완료 (localhost:19092)"
    fi

    # 토픽 생성
    if [ "$REDPANDA_RUNNING" = true ]; then
        echo "  📌 이벤트 토픽 생성 중..."
        TOPICS=(
            "agent.events"
            "agent.task.assigned"
            "agent.task.completed"
            "agent.tool.invoked"
            "agent.tool.completed"
            "agent.error"
            "audit.log"
        )

        for topic in "${TOPICS[@]}"; do
            rpk topic create "$topic" \
                --brokers localhost:19092 \
                --partitions 3 \
                --replicas 1 2>/dev/null || true
        done
        echo "  ✅ 토픽 ${#TOPICS[@]}개 생성 완료"
    fi
fi

# ── .env 파일 준비 ──
echo ""
echo "⚙️  환경 변수 준비..."

if [ ! -f "$PROJECT_DIR/.env" ]; then
    cat > "$PROJECT_DIR/.env" << 'ENVEOF'
# pekko-agent Local Environment (Mac Studio)
# 네이티브: PostgreSQL, Redis, Qdrant
# Docker:   Redpanda (Kafka API 호환)
CLAUDE_API_KEY=sk-ant-your-key-here
CLAUDE_MODEL=claude-sonnet-4-20250514
CLAUDE_MAX_TOKENS=4096
REDIS_URL=redis://localhost:6379
DATABASE_URL=postgres://pekko:pekko_secret@localhost:5432/pekko_agent
KAFKA_BROKERS=localhost:19092
QDRANT_URL=http://localhost:6333
RUST_LOG=info
JWT_SECRET=local-dev-secret
TOKEN_BUDGET_DAILY=1000000
ENVEOF
    echo "  ✅ .env 파일 생성 완료"
    echo "  ⚠️  CLAUDE_API_KEY를 실제 키로 변경해주세요!"
else
    echo "  ✅ .env 파일 이미 존재"
fi

# ── 완료 ──
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 로컬 환경 셋업 완료!"
echo ""
echo "📍 서비스 현황:"
echo "  PostgreSQL : localhost:5432/pekko_agent  (네이티브)"
echo "  Redis      : localhost:6379              (네이티브)"
echo "  Qdrant     : localhost:6333              (네이티브)"
echo "  Redpanda   : localhost:19092             (Docker)"
echo ""
echo "📍 실행:"
echo "  cd $PROJECT_DIR"
echo "  source .env && cargo run -p api-gateway"
echo ""
echo "📍 테스트:"
echo "  curl http://localhost:8080/api/health | jq"
echo ""
echo "📍 중지:"
echo "  brew services stop redis"
echo "  kill \$(cat /tmp/qdrant.pid) 2>/dev/null"
echo "  docker stop pekko-redpanda && docker rm pekko-redpanda"
