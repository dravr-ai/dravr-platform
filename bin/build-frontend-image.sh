#!/bin/bash
# ABOUTME: Builds the Dravr admin frontend Docker image locally for testing only
# ABOUTME: For CI publishing, use .github/workflows/publish-images.yml which pushes to the central Artifact Registry

set -e

# Colors for output
RED='\033[0;31m'
GREEN='\033[0;32m'
BLUE='\033[0;34m'
NC='\033[0m'

# Find project root
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PROJECT_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

IMAGE_TAG="pierre-frontend:local"
VITE_API_BASE_URL="${1:-http://localhost:8081}"

echo -e "${BLUE}=== Pierre Frontend Docker Build ===${NC}"
echo -e "Project root: ${PROJECT_ROOT}"
echo -e "API base URL: ${VITE_API_BASE_URL}"
echo -e "Image tag:    ${IMAGE_TAG}"
echo ""

# Verify Dockerfile exists
DOCKERFILE="$PROJECT_ROOT/docker/images/frontend/Dockerfile"
if [ ! -f "$DOCKERFILE" ]; then
    echo -e "${RED}ERROR: Dockerfile not found at ${DOCKERFILE}${NC}"
    exit 1
fi

echo -e "${BLUE}Building image...${NC}"
docker build \
    -f "$DOCKERFILE" \
    --build-arg "VITE_API_BASE_URL=${VITE_API_BASE_URL}" \
    -t "$IMAGE_TAG" \
    "$PROJECT_ROOT"

echo ""
echo -e "${GREEN}Build complete!${NC}"
echo -e "Image: ${IMAGE_TAG}"
docker images "$IMAGE_TAG" --format "Size: {{.Size}}"
echo ""
echo -e "Run with: docker run --rm -p 8080:8080 ${IMAGE_TAG}"
