# Homebrew formula for logana.
#
# This file is the template used by the release workflow to update the
# pauloremoli/homebrew-logana tap. The VERSION_PLACEHOLDER and
# SHA256_*_PLACEHOLDER tokens are replaced by the release CI.
#
# To install from the tap:
#   brew tap pauloremoli/logana
#   brew install logana
#
# To install directly without tapping:
#   brew install pauloremoli/logana/logana

class Logana < Formula
  desc "A fast, keyboard-driven terminal log viewer and analyzer with filtering, search, and annotations."
  homepage "https://github.com/pauloremoli/logana"
  version "VERSION_PLACEHOLDER"
  license "GPL-3.0-only"

  on_macos do
    on_intel do
      url "https://github.com/pauloremoli/logana/releases/download/v#{version}/logana-v#{version}-x86_64-apple-darwin.tar.gz"
      sha256 "SHA256_MAC_X86_PLACEHOLDER"
    end
    on_arm do
      url "https://github.com/pauloremoli/logana/releases/download/v#{version}/logana-v#{version}-aarch64-apple-darwin.tar.gz"
      sha256 "SHA256_MAC_ARM_PLACEHOLDER"
    end
  end

  on_linux do
    on_intel do
      url "https://github.com/pauloremoli/logana/releases/download/v#{version}/logana-v#{version}-x86_64-unknown-linux-gnu.tar.gz"
      sha256 "SHA256_LINUX_PLACEHOLDER"
    end
  end

  def install
    bin.install "logana"
  end

  test do
    assert_match version.to_s, shell_output("#{bin}/logana --version")
  end
end
