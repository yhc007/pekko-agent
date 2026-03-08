#!/bin/bash
# pekko-agent 로컬 셋업 (Mac)
# ============================================================
# 임베디드 인프라:
#   coreDB       → 내장 (별도 설치 불필요)
#   pekko-event-bus → 내장 (별도 설치 불필요)
# 외부 서비스:
#   Qdrant       → brew install qdrant (벡터 검색용)
# ============================================================
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "$0")" && pwd)"
PROJECT_DIR="$(dirname "$SCRIPT_DIR")"

echo "🚀 pekko-agent 로컬 환경 셋업"
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "  coreDB       : 임베디드 (설치 불필요)"
echo "  EventBus     : 임베디드 (설치 불필요)"
echo "  Qdrant       : 로컬 바이너리 (brew)"

# ── 1. Qdrant 설치 ──
echo ""
echo "📦 [1/3] Qdrant 확인..."

if command -v qdrant &>/dev/null; then
    echo "  ✅ Qdrant 이미 설치됨"
elif command -v brew &>/dev/null; then
    echo "  📌 Qdrant 설치 중..."
    brew install qdrant
    echo "  ✅ Qdrant 설치 완료"
else
    echo "  ⚠️  Homebrew가 없습니다. 수동 설치 필요:"
    echo "     https://qdrant.tech/documentation/guides/installation/"
fi

# ── 2. 데이터 디렉토리 준비 ──
echo ""
echo "📁 [2/3] 데이터 디렉토리 준비..."

mkdir -p "$PROJECT_DIR/data/coredb"
mkdir -p "$PROJECT_DIR/data/qdrant"
echo "  ✅ data/coredb  (coreDB 데이터)"
echo "  ✅ data/qdrant  (Qdrant 벡터 데이터)"

# ── 3. .env 파일 준비 ──
echo ""
echo "⚙️  [3/3] 환경 설정 파일..."

if [ ! -f "$PROJECT_DIR/.env" ]; then
    cp "$PROJECT_DIR/.env.example" "$PROJECT_DIR/.env"
    echo "  ✅ .env 생성 (.env.example 복사)"
    echo "  ⚠️  CLAUDE_API_KEY를 실제 키로 변경하세요!"
else
    echo "  ✅ .env 이미 존재"
fi

# ── 완료 ──
echo ""
echo "━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━━"
echo "✅ 셋업 완료!"
echo ""
echo "📍 실행 방법:"
echo "  ./scripts/run-local.sh start"
echo ""
echo "📍 개별 서비스 실행:"
echo "  ./scripts/run-local.sh start-one api-gateway"
echo ""
echo "📍 상태 확인:"
echo "  ./scripts/run-local.sh status"
