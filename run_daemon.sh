#!/bin/sh

pkill devitd
pkill mcp-server

DEVITD_BINARY=/home/naskel/workspace/devIt/dist/pkg-gnu/devitd \
DEVIT_NOTIFY_HOOK=/home/naskel/bin/ping_claude \
RUST_LOG=info \
/home/naskel/workspace/devIt/dist/pkg-gnu/mcp-server \
--worker-mode \
--worker-id test-worker \
--daemon-socket /tmp/devitd.sock \
--secret 0143c321920e55bd9b17bb0d5ac8543c6fa0200961803c3ff01598e4e6f4007b \
--working-dir /home/naskel/workspace/devIt/ \
--log-level info


