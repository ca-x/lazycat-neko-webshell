#!/usr/bin/env bash
set -euo pipefail

android_api="${NEKO_GHOSTTY_ANDROID_API:-28}"
if [[ ! "$android_api" =~ ^[0-9]+$ ]]; then
    printf '%s\n' 'NEKO_GHOSTTY_ANDROID_API must be an Android API number' >&2
    exit 2
fi

arguments=()
for argument in "$@"; do
    case "$argument" in
        -Dtarget=aarch64-linux-android|-Dtarget=x86_64-linux-android)
            arguments+=("${argument}.${android_api}")
            ;;
        *) arguments+=("$argument") ;;
    esac
done

exec "${NEKO_GHOSTTY_ZIG:-zig}" "${arguments[@]}"
