# Homebrew formula. The release workflow fills the tag and checksum.
# Until a release exists, install from a checkout:
#   cargo install --path . --locked
class Depgraph < Formula
  desc "Self-hosted dependency graph for runtime dependencies and deployments"
  homepage "https://github.com/DennisvDijk/dependency-graph"
  version "0.1.0"
  license "UNLICENSED"
  depends_on "rust" => :build

  on_macos do
    on_arm do
      url "https://github.com/DennisvDijk/dependency-graph/releases/download/v0.1.0/depgraph-aarch64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
    on_intel do
      url "https://github.com/DennisvDijk/dependency-graph/releases/download/v0.1.0/depgraph-x86_64-apple-darwin.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000000"
    end
  end
  on_linux do
    url "https://github.com/DennisvDijk/dependency-graph/releases/download/v0.1.0/depgraph-x86_64-unknown-linux-gnu.tar.gz"
    sha256 "0000000000000000000000000000000000000000000000000000000000000000"
  end

  def install
    bin.install "depgraph"
  end

  test do
    assert_match "depgraph", shell_output("#{bin}/depgraph --version")
  end
end
