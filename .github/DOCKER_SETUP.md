# Quick Setup: Docker Publishing

Follow these steps to enable automated Docker image publishing.

## Checklist

### 1. Docker Hub Setup (5 minutes)

- [ ] Create Docker Hub account at [hub.docker.com](https://hub.docker.com)
- [ ] Navigate to: Account Settings → Security → New Access Token
- [ ] Create token with "Read, Write, Delete" permissions
- [ ] Save token description: `GitHub Actions - Rift`
- [ ] Copy the generated token (you won't see it again!)

### 2. GitHub Secrets Setup (2 minutes)

- [ ] Go to: GitHub Repository → Settings → Secrets and variables → Actions
- [ ] Add secret: `DOCKERHUB_USERNAME` = your Docker Hub username
- [ ] Add secret: `DOCKERHUB_TOKEN` = token from step 1

### 3. Test the Workflow (1 minute)

- [ ] Push code to `main` branch OR
- [ ] Go to Actions → Build and Publish Docker Image → Run workflow

### 4. Verify Published Images

- [ ] Check Docker Hub: `https://hub.docker.com/r/<username>/rift-proxy`
- [ ] Check GHCR: `https://github.com/<username>/rift/pkgs/container/rift-proxy`

### 5. (Optional) Make GHCR Public

- [ ] Go to GitHub repository → Packages tab
- [ ] Click `rift-proxy` package
- [ ] Package settings → Change visibility → Public

## Pull Your Published Image

```bash
# From Docker Hub
docker pull <your-username>/rift-proxy:latest

# From GitHub Container Registry
docker pull ghcr.io/<your-username>/rift-proxy:latest
```

## Need More Details?

See the full guide: [docs/DOCKER_PUBLISHING.md](../docs/DOCKER_PUBLISHING.md)

## Common Issues

**"Invalid credentials" error**
→ Verify `DOCKERHUB_USERNAME` and `DOCKERHUB_TOKEN` are correct

**"Package not found" on GHCR**
→ Make package public (step 5) or authenticate with Docker

**Build takes too long**
→ First build is ~10-15 minutes. Subsequent builds use cache and are faster.
