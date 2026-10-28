class Lilia < Formula
  desc "Local-first multimodel embedded and daemon database (CLI, Daemon & MCP)"
  homepage "https://liliadb.org"
  version "0.1.0-alpha.3"
  license "Apache-2.0"

  on_macos do
    if Hardware::CPU.arm?
      url "https://github.com/shaynlink/liliadb/releases/download/v#{version}/lilia-darwin-arm64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000001"
    else
      url "https://github.com/shaynlink/liliadb/releases/download/v#{version}/lilia-darwin-x64.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000002"
    end
  end

  on_linux do
    if Hardware::CPU.arm?
      url "https://github.com/shaynlink/liliadb/releases/download/v#{version}/lilia-linux-arm64-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000003"
    else
      url "https://github.com/shaynlink/liliadb/releases/download/v#{version}/lilia-linux-x64-gnu.tar.gz"
      sha256 "0000000000000000000000000000000000000000000000000000000000000004"
    end
  end

  def install
    bin.install "lilia"
    bin.install "lilia-daemon"
    bin.install "lilia-mcp"
  end

  service do
    run [opt_bin/"lilia-daemon", "--database", var/"liliadb/database.lilia", "--http-port", "4444"]
    keep_alive true
    working_dir var/"liliadb"
    log_path var/"log/lilia-daemon.log"
    error_log_path var/"log/lilia-daemon.log"
  end

  test do
    assert_match "lilia", shell_output("#{bin}/lilia --help")
  end
end
