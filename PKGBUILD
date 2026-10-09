# Maintainer: EMeowAGENT <ccedercatcc@gmail.com>
pkgname=nekoya-stt
pkgver=0.1.0
pkgrel=1
pkgdesc="Speech-to-text daemon: Qwen realtime ASR feeding clipboard / VRChat OSC / OpenCode, with a WayVR subtitle panel"
arch=('x86_64')
url="https://github.com/EMeowAGENT/nekoya-stt-daemon"
license=('MIT')
depends=('gcc-libs')
makedepends=('cargo')

# 本仓库即源码：直接从 PKGBUILD 所在目录（$startdir）构建。
# 只安装：可执行文件 + LICENSE + systemd 用户单元。面板 XML 仅作模板，不安装。
source=()
sha256sums=()

build() {
  cd "$startdir"
  export CARGO_HOME="${CARGO_HOME:-$startdir/.cargo-home}"
  cargo build --release --locked
}

package() {
  cd "$startdir"
  install -Dm755 "target/release/nekoya-stt" "$pkgdir/usr/bin/nekoya-stt"
  install -Dm644 "LICENSE" "$pkgdir/usr/share/licenses/$pkgname/LICENSE"
  install -Dm644 "systemd/nekoya-stt.service" "$pkgdir/usr/lib/systemd/user/$pkgname.service"
}
