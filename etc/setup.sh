#!/bin/sh

set -e

if id rockslide >/dev/null 2>&1; then
  echo "rockslide user already setup, not recreating"
else
  groupadd -r rockslide
  useradd -m -g rockslide -d /var/lib/rockslide rockslide
  chmod 0700 /var/lib/rockslide
fi;

systemctl daemon-reload
systemctl restart rockslide
