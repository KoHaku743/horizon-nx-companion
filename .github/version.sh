#!/bin/sh
# The version builds carry: the tag's (v0.1.0 → 0.1.0), else a development number.
if [ -n "${RELEASE:-}" ]; then echo "$RELEASE"; exit 0; fi
case "${GITHUB_REF:-}" in
  refs/tags/v*) echo "${GITHUB_REF_NAME#v}" ;;
  *) echo "0.2.0-dev.${GITHUB_RUN_NUMBER:-0}" ;;
esac
