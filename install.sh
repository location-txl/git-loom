#!/bin/sh
set -eu

offer_zsh_completion() {
    login_shell=${SHELL:-}
    [ "${login_shell##*/}" = zsh ] && [ -z "${CI:-}" ] && [ -t 1 ] || return 0
    # stdin may carry the installer itself (curl | sh); read from the real terminal.
    ( : </dev/tty ) 2>/dev/null || return 0
    [ "$("$1" completions zsh --status 2>/dev/null)" = missing ] || return 0
    printf '要安装 Zsh 补全吗？[y/N] ' >&2
    IFS= read -r answer </dev/tty || return 0
    case "$answer" in
        [yY]|[yY][eE][sS])
            if ! "$1" completions zsh --install; then
                echo 'Zsh 补全安装失败；git-loom 本体已安装。' >&2
            fi
            ;;
    esac
}

# 函数完整下载后才开始安装，避免管道下载中断时执行不完整脚本。
main() {
    if [ "$(uname -s)" != Darwin ]; then
        echo '错误：目前仅支持 macOS Apple Silicon。' >&2
        exit 1
    fi
    if [ "$(uname -m)" != arm64 ] && [ "$(sysctl -in sysctl.proc_translated 2>/dev/null || true)" != 1 ]; then
        echo '错误：目前仅支持 Apple Silicon，不支持 Intel Mac。' >&2
        exit 1
    fi
    version=${GIT_LOOM_VERSION:-latest}
    install_dir=${GIT_LOOM_INSTALL_DIR:-"$HOME/.local/bin"}
    case "$version" in
        latest) release_path=latest/download ;;
        v[0-9]*)
            case "$version" in *[!a-zA-Z0-9._-]*) echo '错误：无效版本号。' >&2; exit 1 ;; esac
            release_path=download/$version ;;
        *) echo '错误：版本号须以 v 开头，例如 v0.2.0。' >&2; exit 1 ;;
    esac
    case "$install_dir" in
        /*) ;;
        *) echo '错误：安装目录必须是绝对路径。' >&2; exit 1 ;;
    esac
    asset=git-loom-aarch64-apple-darwin.tar.gz
    base_url=https://github.com/location-txl/git-loom/releases/$release_path
    tmp_dir=$(mktemp -d)
    trap 'rm -rf "$tmp_dir"' EXIT
    trap 'exit 1' HUP INT TERM
    echo "正在下载 git-loom ($version)…"
    curl -fSL --proto '=https' --tlsv1.2 "$base_url/$asset" -o "$tmp_dir/$asset"
    curl -fSL --proto '=https' --tlsv1.2 "$base_url/$asset.sha256" -o "$tmp_dir/$asset.sha256"
    (cd "$tmp_dir" && shasum -a 256 -c "$asset.sha256")
    tar -xzf "$tmp_dir/$asset" -C "$tmp_dir" git-loom
    chmod 755 "$tmp_dir/git-loom"
    "$tmp_dir/git-loom" --version
    mkdir -p "$install_dir"
    if [ -d "$install_dir/git-loom" ]; then
        echo '错误：目标 git-loom 是目录。' >&2
        exit 1
    fi
    # 在目标文件系统中准备好文件再替换，下载失败不会破坏旧版本。
    staged=$(mktemp "$install_dir/.git-loom.XXXXXX")
    trap 'rm -rf "$tmp_dir"; rm -f "$staged"' EXIT
    install -m 755 "$tmp_dir/git-loom" "$staged"
    mv -f "$staged" "$install_dir/git-loom"
    echo "已安装：$install_dir/git-loom"
    case ":$PATH:" in
        *":$install_dir:"*) echo '运行 git loom --help 开始使用。' ;;
        *)
            echo '请将安装目录加入 PATH（可写入 ~/.zshrc），然后运行 git loom --help：'
            # Print a command for the user to run later, not this process's PATH.
            # shellcheck disable=SC2016
            printf '  export PATH="%s:$PATH"\n' "$install_dir"
            ;;
    esac
    offer_zsh_completion "$install_dir/git-loom"
}

main "$@"
