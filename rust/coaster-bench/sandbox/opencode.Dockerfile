# podman build -t localhost/coaster-or -f rust/coaster-bench/sandbox/opencode.Dockerfile rust/coaster-bench/sandbox
FROM localhost/coaster-sandbox

RUN npm install -g opencode-ai@latest && opencode --version
