#!/bin/bash
# pekko-agent 로컬 개발 환경 셋업 (Mac Studio)
# 사전 요구: PostgreSQL 설치됨, Docker Desktop 설치됨
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "🚀 pekko-agent 로컬 환경 셋업 시작"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"

# ── 1. PostgreSQL 데이터베이스 준비 ──
echo ""
echo "📦 [1/4] PostgreSQL 데이터베이스 준비..."

# DB 존재 여부 확인
if psql -lqt | cut -d \| -f 1 | grep -qw pekko_agent; then
    echo "  ✅ 'pekko_agent' DB 이미 존재"
else
    echo "  📌 'pekko_agent' DB 생성 중..."
    createdb pekko_agent 2>/dev/null || true

    # pekko 유저 생성 (이미 있으면 무시)
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

    echo "  ✅ DB 및 유저 생성 완료"
fi

# 스키마 적용
echo "  📌 스키마 적용 중..."
psql -d pekko_agent -f "$PROJECT_DIR/infra/init.sql" 2>/dev/null || {
    echo "  ⚠️  스키마 이미 적용됨 (중복 무시)"
}

# pekko 유저에게 테이블 권한 부여
psql -d pekko_agent -c "
    GRANT ALL PRIVILEGES ON ALL TABLES IN SCHEMA public TO pekko;
    GRANT ALL PRIVILEGES ON ALL SEQUENCES IN SCHEMA public TO pekko;
    ALTER DEFAULT PRIVILEGES IN SCHEMA public GRANT ALL ON TABLES TO pekko;
" 2>/dev/null || true

echo "  ✅ PostgreSQL 준비 완료"

# ── 2. Docker 서비스 기동 (Redis, Qdrant, Redpanda) ──
echo ""
echo "🐳 [2/4] Docker 서비스 기동 (Redis, Qdrant, Redpanda)..."

cd "$PROJECT_DIR"
docker compose -f docker-compose-local.yml up -d

echo "  ⏳ 서비스 헬스체크 대기 중..."
sleep 5

# 헬스체크
echo -n "  Redis: "
docker compose -f docker-compose-local.yml exec -T redis redis-cli ping 2>/dev/null && echo "" || echo "❌ 연결 실패"

echo -n "  Qdrant: "
curl -sf http://localhost:6333/healthz > /dev/null && echo "✅ 정상" || echo "❌ 연결 실패"

echo -n "  Redpanda: "
docker compose -f docker-compose-local.yml exec -T redpanda rpk cluster health 2>/dev/null | head -1 || echo "❌ 연결 실패"

# ── 3. Redpanda 토픽 생성 ──
echo ""
echo "📨 [3/4] Redpanda 토픽 생성..."

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
    docker compose -f docker-compose-local.yml exec -T redpanda \
        rpk topic create "$topic" --partitions 3 --replicas 1 2>/dev/null || true
    echo "  ✅ $topic"
done

# ── 4. .env 파일 준비 ──
echo ""
echo "⚙️  [4/4] 환경 변수 준비..."

if [ ! -f "$PROJECT_DIR/.env" ]; then
    cp "$PROJECT_DIR/.env.local" "$PROJECT_DIR/.env"
    echo "  ✅ .env.local → .env 복사됨"
    echo "  ⚠️  .env 파일에서 CLAUDE_API_KEY를 설정해주세요!"
else
    echo "  ✅ .env 파일 이미 존재"
fi

# ── 완료 ──
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 로컬 환경 셋업 완료!"
echo ""
echo "📍 서비스 접속 정보:"
echo "  PostgreSQL : localhost:5432/pekko_agent (pekko/pekko_secret)"
echo "  Redis      : localhost:6379"
echo "  Qdrant     : http://localhost:6333"
echo "  Redpanda   : localhost:19092 (Kafka API)"
echo "  RP Console : http://localhost:8888"
echo ""
echo "📍 실행 방법:"
echo "  # API Gateway 실행"
echo "  source .env && cargo run -p api-gateway"
echo ""
echo "  # EHS Agent 실행 (별도 터미널)"
echo "  source .env && cargo run -p ehs-permit-agent"
echo "  source .env && cargo run -p ehs-inspection-agent"
echo "  source .env && cargo run -p ehs-compliance-agent"
echo ""
echo "  # API 테스트"
echo "  curl http://localhost:8080/api/health | jq"
