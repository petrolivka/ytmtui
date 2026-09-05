# Homebrew formula. Install with:
#   brew install --HEAD --formula ./contrib/packaging/ytmtui.rb
class Ytmtui < Formula
  desc "YouTube Music client for the terminal, with a live audio spectrum"
  homepage "https://github.com/petrolivka/ytmtui"
  url "https://github.com/petrolivka/ytmtui/archive/refs/tags/v0.1.0.tar.gz"
  sha256 "REPLACE_ON_RELEASE"
  license "GPL-3.0-or-later"
  head "https://github.com/petrolivka/ytmtui.git", branch: "master"

  depends_on "rust" => :build
  # ffmpeg decodes and yt-dlp resolves stream URLs; the app refuses to work
  # usefully without either, so they are runtime dependencies rather than hints.
  depends_on "ffmpeg"
  depends_on "yt-dlp"

  def install
    system "cargo", "install", *std_cargo_args(path: ".")
  end

  test do
    assert_match "ytmtui", shell_output("#{bin}/ytmtui --version")
    # --doctor exits cleanly even with nothing configured.
    system bin/"ytmtui", "--doctor"
  end
end
