#!/usr/bin/env bash
# MyLianPC Linux one-command installer / updater.
#
#   curl -fsSL https://raw.githubusercontent.com/abdh01h/MyLianPC-Linux/HEAD/install.sh | sudo bash
#
# Running it again updates an existing installation: it pulls the latest source,
# rebuilds and restarts the service. The installed updater is also available as
# `sudo mylianpc-update`.
#
# Environment overrides:
#   MYLIANPC_REPO_URL   Git repository to clone (default: the GitHub project)
#   MYLIANPC_SRC_DIR    Where to keep the source checkout (default: /opt/mylianpc-src)
set -Eeuo pipefail

REPO_URL="${MYLIANPC_REPO_URL:-https://github.com/abdh01h/MyLianPC-Linux.git}"
SRC_DIR="${MYLIANPC_SRC_DIR:-/opt/mylianpc-src}"
MIN_RUST="1.85"

say() { printf '\n==> %s\n' "$*"; }
fail() { printf 'error: %s\n' "$*" >&2; exit 1; }
trap 'printf "error: installation stopped at line %s\n" "$LINENO" >&2' ERR

if [[ $EUID -ne 0 ]]; then
    fail "run this script as root (for example: curl … | sudo bash)"
fi

# The service is a systemd unit and the installer writes to /usr and /etc.
# Arch derivatives on OpenRC (Artix) and immutable images (SteamOS) need
# different packaging, so stop with a clear message instead of halfway through.
if [[ ! -d /run/systemd/system ]] || ! command -v systemctl >/dev/null 2>&1; then
    fail "this installer requires systemd. Arch derivatives using OpenRC (for example Artix) are not supported."
fi
[[ -w /usr ]] || fail "/usr is not writable. Immutable/read-only systems such as SteamOS need a writable root or an overlay."

# ---------------------------------------------------------------- source tree
# A checkout passed directly (./install.sh) is used as-is. A piped or installed
# copy clones/updates a stable source tree under /opt.
SCRIPT_DIR=""
if [[ -n "${BASH_SOURCE[0]:-}" && -f "${BASH_SOURCE[0]}" ]]; then
    SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
fi
if [[ -n "$SCRIPT_DIR" && -f "$SCRIPT_DIR/Cargo.toml" && -d "$SCRIPT_DIR/crates" ]]; then
    SOURCE="$SCRIPT_DIR"
    LOCAL_CHECKOUT=1
else
    SOURCE="$SRC_DIR"
    LOCAL_CHECKOUT=0
fi

# ------------------------------------------------------------- prerequisites
have_gtk() {
    command -v pkg-config >/dev/null 2>&1 &&
        pkg-config --exists gtk4 &&
        pkg-config --atleast-version=4.10 gtk4
}

missing_build_tools() {
    ! command -v cc >/dev/null 2>&1 ||
        ! command -v git >/dev/null 2>&1 ||
        ! command -v curl >/dev/null 2>&1 ||
        ! command -v pkg-config >/dev/null 2>&1 ||
        ! have_gtk
}

install_build_tools() {
    say "Installing build tools and GTK 4 development packages"
    if command -v apt-get >/dev/null 2>&1; then
        export DEBIAN_FRONTEND=noninteractive
        apt-get update -qq
        apt-get install -y --no-install-recommends \
            build-essential pkg-config libgtk-4-dev git curl ca-certificates
    elif command -v dnf >/dev/null 2>&1; then
        dnf install -y gcc gcc-c++ make pkgconf-pkg-config gtk4-devel git curl ca-certificates
    elif command -v pacman >/dev/null 2>&1; then
        # Arch does not split -devel packages, so gtk4 itself carries the
        # headers. Do not run `pacman -Sy pkg`: a partial upgrade is unsupported
        # and a stale sync database makes the install fail with 404s. Point the
        # user at a full upgrade and retry instead.
        if [[ -n "$(pacman -Qu 2>/dev/null)" ]]; then
            printf 'warning: pending package upgrades found; Arch recommends "sudo pacman -Syu" before installing.\n' >&2
        fi
        if ! pacman -S --needed --noconfirm base-devel pkgconf gtk4 git curl ca-certificates; then
            fail "pacman could not install the build dependencies. Run 'sudo pacman -Syu' to update the system, then rerun this installer."
        fi
    elif command -v zypper >/dev/null 2>&1; then
        zypper --non-interactive install gcc gcc-c++ make pkgconf-pkg-config gtk4-devel git curl ca-certificates
    else
        fail "unsupported package manager. Install git, a C compiler, pkg-config and GTK 4.10+ development files, then rerun."
    fi
}

if missing_build_tools; then
    install_build_tools
fi
have_gtk || fail "GTK 4.10 or newer is required. Ubuntu 24.04+, Debian 13+, Fedora 40+, Arch or openSUSE Tumbleweed are supported."

command -v curl >/dev/null 2>&1 || fail "curl is required."
command -v git >/dev/null 2>&1 || fail "git is required."

# ------------------------------------------------------------------ Rust toolchain
# Keep the root toolchain out of the desktop user's Cargo directories.
export CARGO_HOME="${CARGO_HOME:-/root/.cargo}"
export RUSTUP_HOME="${RUSTUP_HOME:-/root/.rustup}"
export PATH="$CARGO_HOME/bin:$PATH"
version_ge() {
    [[ "$(printf '%s\n%s\n' "$2" "$1" | sort -V | head -n1)" == "$2" ]]
}
cargo_version=""
if command -v cargo >/dev/null 2>&1; then
    cargo_version="$(cargo --version | awk '{print $2}')"
fi
if [[ -z "$cargo_version" ]] || ! version_ge "$cargo_version" "$MIN_RUST"; then
    say "Installing the Rust toolchain (minimum $MIN_RUST)"
    curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs |
        sh -s -- -y --profile minimal --default-toolchain stable
    export PATH="${CARGO_HOME:-$HOME/.cargo}/bin:$PATH"
fi
command -v cargo >/dev/null 2>&1 || fail "cargo is still not available after installing Rust."

# --------------------------------------------------------------------- source
if [[ $LOCAL_CHECKOUT -eq 1 ]]; then
    say "Building the local checkout at $SOURCE"
else
    if [[ -d "$SOURCE/.git" ]]; then
        say "Updating source at $SOURCE"
        if [[ -n "$(git -C "$SOURCE" status --porcelain)" ]]; then
            printf 'warning: %s has local changes; building without pulling.\n' "$SOURCE" >&2
        else
            git -C "$SOURCE" fetch --prune origin
            git -C "$SOURCE" pull --ff-only
        fi
    else
        [[ -e "$SOURCE" ]] && fail "$SOURCE exists but is not a git checkout."
        say "Fetching source into $SOURCE"
        install -d "$(dirname "$SOURCE")"
        git clone --depth 1 "$REPO_URL" "$SOURCE"
    fi
fi

# ---------------------------------------------------------------------- build
if [[ -x /usr/bin/mylianpc-ui ]]; then
    say "Existing installation found; updating MyLianPC"
else
    say "Installing MyLianPC"
fi
cd "$SOURCE"
cargo build --release --locked \
    -p mylianpc-service -p mylianpc-ui -p mylianpc-probe

packaging/install.sh

if pgrep -f '^/usr/bin/mylianpc-ui$' >/dev/null 2>&1; then
    printf '\nClose and reopen the MyLianPC window to use the new version.\n'
fi
say "Done. The service is running the new build."
