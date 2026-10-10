# Maintainer: EMeowAGENT <ccendercatcc@gmail.com>
pkgname=nekoya-stt
pkgver=0.1.0
pkgrel=1
pkgdesc="Speech-to-text daemon: Qwen realtime ASR feeding clipboard / VRChat OSC / OpenCode, with a WayVR subtitle panel"
arch=('x86_64')
url="https://github.com/EMeowAGENT/nekoya-stt-daemon"
license=('MIT')
depends=('gcc-libs')
makedepends=('cargo')
# 关键：makepkg 默认的 CFLAGS 带 `-flto=auto`，会被 cc crate 带进 ring 的 C 构建，
# 产出 LTO 位码对象；rustc 最终链接（lld）无法解析 → 全部 `ring_core_*` 未定义。
# 关闭 makepkg 的 LTO 开关，并在 build() 里清空 CFLAGS，避免污染 Rust 构建。
options=('!lto')

# 本仓库即源码：直接从 PKGBUILD 所在目录（$startdir）构建。
# 只安装：可执行文件 + LICENSE + systemd 用户单元。面板 XML 仅作模板，不安装。
source=()
sha256sums=()

build() {
  cd "$startdir"
  # 不使用发行版 CFLAGS/CXXFLAGS/LDFLAGS（Rust 项目不需要，且 -flto 会破坏 ring）。
  export CFLAGS="" CXXFLAGS="" LDFLAGS=""
  export CARGO_HOME="${CARGO_HOME:-$startdir/.cargo-home}"
  cargo build --release --locked
}

package() {
  cd "$startdir"
  install -Dm755 "target/release/nekoya-stt" "$pkgdir/usr/bin/nekoya-stt"
  install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 "systemd/nekoya-stt.service" "$pkgdir/usr/lib/systemd/user/$pkgname.service"
}
