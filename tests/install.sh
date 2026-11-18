#!/bin/sh
# Offline installer tests. Optional argument exercises the actual native binary.
set -eu
root=$(CDPATH='' cd -- "$(dirname -- "$0")/.." && pwd)
work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT
mkdir -p "$work/mocks" "$work/assets" "$work/payload" "$work/home"
export HOME="$work/home"
export FIXTURES="$work/assets"
export CURL_LOG="$work/curl.log"
export GIT_LOOM_INSTALL_DIR="$work/install dir"
export GIT_LOOM_VERSION=v0.2.0
export MOCK_OS=Darwin MOCK_ARCH=arm64 MOCK_TRANSLATED=0 MOCK_CURL_FAIL=0
if [ "$#" -gt 0 ]; then
    cp "$1" "$work/payload/git-loom"
else
    printf '#!/bin/sh\nprintf "git-loom 0.2.0\\n"\n' > "$work/payload/git-loom"
fi
chmod +x "$work/payload/git-loom"
cat > "$work/mocks/uname" <<'MOCK'
#!/bin/sh
case "$1" in -s) echo "$MOCK_OS" ;; -m) echo "$MOCK_ARCH" ;; *) exit 1 ;; esac
MOCK
cat > "$work/mocks/sysctl" <<'MOCK'
#!/bin/sh
echo "$MOCK_TRANSLATED"
MOCK
cat > "$work/mocks/curl" <<'MOCK'
#!/bin/sh
set -eu
[ "$MOCK_CURL_FAIL" = 0 ] || exit 22
url= output=
while [ "$#" -gt 0 ]; do
    case "$1" in
        --proto|--tlsv1.2) if [ "$1" = --proto ]; then shift; fi ;;
        -o) shift; output=$1 ;;
        https://*) url=$1 ;;
    esac
    shift
done
printf '%s\n' "$url" >> "$CURL_LOG"
[ -n "$url" ] && [ -n "$output" ]
cp "$FIXTURES/${url##*/}" "$output"
MOCK
chmod +x "$work/mocks/"*
export PATH="$work/mocks:$PATH"
asset=git-loom-aarch64-apple-darwin.tar.gz
tar -czf "$FIXTURES/$asset" -C "$work/payload" git-loom
(cd "$FIXTURES" && shasum -a 256 "$asset" > "$asset.sha256")
run_install() { sh "$root/install.sh" > "$work/output" 2>&1; }
expect_failure() {
    if run_install; then
        echo 'Expected installer to fail' >&2
        cat "$work/output" >&2
        exit 1
    fi
}
# Fresh install and versioned URL, including a destination containing spaces.
run_install
cmp "$work/payload/git-loom" "$GIT_LOOM_INSTALL_DIR/git-loom"
"$GIT_LOOM_INSTALL_DIR/git-loom" --version
[ "$(wc -l < "$CURL_LOG" | tr -d ' ')" = 2 ]
grep -F 'releases/download/v0.2.0/' "$CURL_LOG" >/dev/null
# Upgrade replaces an existing file only after verification.
printf 'old binary\n' > "$GIT_LOOM_INSTALL_DIR/git-loom"
run_install
cmp "$work/payload/git-loom" "$GIT_LOOM_INSTALL_DIR/git-loom"
# Corruption and HTTP failure must both preserve the previously installed bytes.
printf 'keep this binary\n' > "$GIT_LOOM_INSTALL_DIR/git-loom"
cp "$GIT_LOOM_INSTALL_DIR/git-loom" "$work/old"
printf 'corrupted\n' >> "$FIXTURES/$asset"
expect_failure
cmp "$work/old" "$GIT_LOOM_INSTALL_DIR/git-loom"
export MOCK_CURL_FAIL=1
expect_failure
cmp "$work/old" "$GIT_LOOM_INSTALL_DIR/git-loom"
export MOCK_CURL_FAIL=0
# Rejected inputs/platforms must never download anything.
: > "$CURL_LOG"
export MOCK_OS=Linux
expect_failure
export MOCK_OS=Darwin MOCK_ARCH=x86_64
expect_failure
export MOCK_ARCH=arm64 GIT_LOOM_VERSION='v0.2.0/invalid'
expect_failure
export GIT_LOOM_VERSION=v0.2.0 GIT_LOOM_INSTALL_DIR=relative/path
expect_failure
[ ! -s "$CURL_LOG" ]
# Rosetta is accepted; latest selects GitHub's stable release endpoint.
export MOCK_ARCH=x86_64 MOCK_TRANSLATED=1 GIT_LOOM_VERSION=latest
export GIT_LOOM_INSTALL_DIR="$work/rosetta"
tar -czf "$FIXTURES/$asset" -C "$work/payload" git-loom
(cd "$FIXTURES" && shasum -a 256 "$asset" > "$asset.sha256")
run_install
cmp "$work/payload/git-loom" "$GIT_LOOM_INSTALL_DIR/git-loom"
grep -F 'releases/latest/download/' "$CURL_LOG" >/dev/null
# A directory at the binary path must be preserved, not overwritten.
export GIT_LOOM_INSTALL_DIR="$work/directory-target"
mkdir -p "$GIT_LOOM_INSTALL_DIR/git-loom"
printf 'keep\n' > "$GIT_LOOM_INSTALL_DIR/git-loom/sentinel"
expect_failure
[ "$(cat "$GIT_LOOM_INSTALL_DIR/git-loom/sentinel")" = keep ]
echo 'Installer smoke tests passed'
