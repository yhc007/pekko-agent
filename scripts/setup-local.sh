#!/bin/bash
# ============================================================
# [DEPRECATED] 이 스크립트는 더 이상 사용되지 않습니다.
#
# 새 스크립트를 사용하세요:
#   ./scripts/setup-native.sh   # 초기 셋업
#   ./scripts/run-local.sh      # 실행/중지/상태 관리
#
# 변경 사항:
#   - PostgreSQL → coreDB (임베디드)
#   - Redis → coreDB (임베디드)
#   - Redpanda/Kafka → pekko-event-bus (임베디드)
#   - Docker → 불필요 (Qdrant만 로컬 바이너리)
# ============================================================

echo "⚠️  이 스크립트는 더 이상 사용되지 않습니다."
echo ""
echo "새 스크립트를 사용하세요:"
echo "  ./scripts/setup-native.sh   # 초기 셋업"
echo "  ./scripts/run-local.sh      # 실행/중지/상태 관리"
exit 1
