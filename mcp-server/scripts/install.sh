#!/bin/bash
# IronBase MCP Server Linux/macOS Installer
#
# Usage:
#   sudo ./install.sh              # Install service
#   sudo ./install.sh --uninstall  # Uninstall service

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
NC='\033[0m' # No Color

# Configuration
SERVICE_NAME="ironbase"
DISPLAY_NAME="IronBase MCP Server"

# Detect OS
detect_os() {
    if [[ "$OSTYPE" == "linux-gnu"* ]]; then
        echo "linux"
    elif [[ "$OSTYPE" == "darwin"* ]]; then
        echo "macos"
    else
        echo "unknown"
    fi
}

OS=$(detect_os)

# Linux paths
LINUX_INSTALL_DIR="/usr/local/bin"
LINUX_DATA_DIR="/var/lib/ironbase"
LINUX_CONFIG_DIR="/etc/ironbase"
LINUX_LOG_DIR="/var/log/ironbase"

# macOS paths
MACOS_INSTALL_DIR="/usr/local/bin"
MACOS_DATA_DIR="/usr/local/var/ironbase"
MACOS_CONFIG_DIR="/usr/local/etc/ironbase"
MACOS_LOG_DIR="/usr/local/var/log/ironbase"

# Check root
check_root() {
    if [[ $EUID -ne 0 ]]; then
        echo -e "${RED}ERROR: This script must be run as root (use sudo)${NC}"
        exit 1
    fi
}

# Find executable
find_executable() {
    local script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
    local exe_name="mcp-ironbase-server"

    # Try relative paths
    local paths=(
        "$script_dir/../target/release/$exe_name"
        "./target/release/$exe_name"
        "../target/release/$exe_name"
    )

    for path in "${paths[@]}"; do
        if [[ -f "$path" ]]; then
            echo "$(cd "$(dirname "$path")" && pwd)/$(basename "$path")"
            return 0
        fi
    done

    return 1
}

# Install on Linux (systemd)
install_linux() {
    echo -e "${CYAN}=== IronBase MCP Server Installation (Linux) ===${NC}"
    echo ""

    local exe_path
    exe_path=$(find_executable) || {
        echo -e "${RED}ERROR: Cannot find mcp-ironbase-server executable${NC}"
        echo -e "${YELLOW}Please build the project first: cargo build --release${NC}"
        exit 1
    }

    echo -e "${GREEN}Source executable: $exe_path${NC}"

    # Create ironbase user if not exists
    if ! id -u ironbase >/dev/null 2>&1; then
        echo "Creating ironbase user..."
        useradd -r -s /bin/false -d "$LINUX_DATA_DIR" ironbase || true
    fi

    # Create directories
    echo "Creating directories..."
    mkdir -p "$LINUX_INSTALL_DIR"
    mkdir -p "$LINUX_DATA_DIR"
    mkdir -p "$LINUX_CONFIG_DIR"
    mkdir -p "$LINUX_LOG_DIR"

    # Copy executable
    echo "Installing executable..."
    cp "$exe_path" "$LINUX_INSTALL_DIR/mcp-ironbase-server"
    chmod 755 "$LINUX_INSTALL_DIR/mcp-ironbase-server"

    # Create default config
    if [[ ! -f "$LINUX_CONFIG_DIR/config.toml" ]]; then
        echo "Creating default configuration..."
        cat > "$LINUX_CONFIG_DIR/config.toml" << 'EOF'
# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080

[database]
path = "/var/lib/ironbase/data.mlite"

[logging]
level = "info"
EOF
    fi

    # Set ownership
    chown -R ironbase:ironbase "$LINUX_DATA_DIR"
    chown -R ironbase:ironbase "$LINUX_LOG_DIR"

    # Install systemd service using built-in command
    echo "Installing systemd service..."
    "$LINUX_INSTALL_DIR/mcp-ironbase-server" install || {
        echo -e "${YELLOW}Note: Service installation requires the binary to be run with appropriate permissions${NC}"
    }

    echo ""
    echo -e "${GREEN}=== Installation Complete ===${NC}"
    echo ""
    echo -e "Service Name:    ${SERVICE_NAME}"
    echo -e "Install Dir:     ${LINUX_INSTALL_DIR}"
    echo -e "Data Dir:        ${LINUX_DATA_DIR}"
    echo -e "Config File:     ${LINUX_CONFIG_DIR}/config.toml"
    echo -e "Log Dir:         ${LINUX_LOG_DIR}"
    echo ""
    echo -e "${CYAN}Commands:${NC}"
    echo "  Start service:   sudo systemctl start $SERVICE_NAME"
    echo "  Stop service:    sudo systemctl stop $SERVICE_NAME"
    echo "  Check status:    sudo systemctl status $SERVICE_NAME"
    echo "  View logs:       sudo journalctl -u $SERVICE_NAME -f"
    echo ""
    echo -e "${CYAN}Or use the built-in commands:${NC}"
    echo "  mcp-ironbase-server start"
    echo "  mcp-ironbase-server stop"
    echo "  mcp-ironbase-server status"
}

# Install on macOS (launchd)
install_macos() {
    echo -e "${CYAN}=== IronBase MCP Server Installation (macOS) ===${NC}"
    echo ""

    local exe_path
    exe_path=$(find_executable) || {
        echo -e "${RED}ERROR: Cannot find mcp-ironbase-server executable${NC}"
        echo -e "${YELLOW}Please build the project first: cargo build --release${NC}"
        exit 1
    }

    echo -e "${GREEN}Source executable: $exe_path${NC}"

    # Create directories
    echo "Creating directories..."
    mkdir -p "$MACOS_INSTALL_DIR"
    mkdir -p "$MACOS_DATA_DIR"
    mkdir -p "$MACOS_CONFIG_DIR"
    mkdir -p "$MACOS_LOG_DIR"

    # Copy executable
    echo "Installing executable..."
    cp "$exe_path" "$MACOS_INSTALL_DIR/mcp-ironbase-server"
    chmod 755 "$MACOS_INSTALL_DIR/mcp-ironbase-server"

    # Create default config
    if [[ ! -f "$MACOS_CONFIG_DIR/config.toml" ]]; then
        echo "Creating default configuration..."
        cat > "$MACOS_CONFIG_DIR/config.toml" << 'EOF'
# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080

[database]
path = "/usr/local/var/ironbase/data.mlite"

[logging]
level = "info"
EOF
    fi

    # Install launchd service using built-in command
    echo "Installing launchd service..."
    "$MACOS_INSTALL_DIR/mcp-ironbase-server" install || {
        echo -e "${YELLOW}Note: Service installation requires the binary to be run with appropriate permissions${NC}"
    }

    echo ""
    echo -e "${GREEN}=== Installation Complete ===${NC}"
    echo ""
    echo -e "Service Name:    com.ironbase.server"
    echo -e "Install Dir:     ${MACOS_INSTALL_DIR}"
    echo -e "Data Dir:        ${MACOS_DATA_DIR}"
    echo -e "Config File:     ${MACOS_CONFIG_DIR}/config.toml"
    echo -e "Log Dir:         ${MACOS_LOG_DIR}"
    echo ""
    echo -e "${CYAN}Commands:${NC}"
    echo "  Start service:   sudo launchctl start com.ironbase.server"
    echo "  Stop service:    sudo launchctl stop com.ironbase.server"
    echo "  Check status:    sudo launchctl list | grep ironbase"
    echo "  View logs:       tail -f $MACOS_LOG_DIR/*.log"
    echo ""
    echo -e "${CYAN}Or use the built-in commands:${NC}"
    echo "  mcp-ironbase-server start"
    echo "  mcp-ironbase-server stop"
    echo "  mcp-ironbase-server status"
}

# Uninstall on Linux
uninstall_linux() {
    echo -e "${CYAN}=== IronBase MCP Server Uninstallation (Linux) ===${NC}"
    echo ""

    # Use built-in uninstall command
    if [[ -f "$LINUX_INSTALL_DIR/mcp-ironbase-server" ]]; then
        "$LINUX_INSTALL_DIR/mcp-ironbase-server" uninstall || true
    fi

    # Stop and disable service
    systemctl stop $SERVICE_NAME 2>/dev/null || true
    systemctl disable $SERVICE_NAME 2>/dev/null || true

    # Remove service file
    rm -f /etc/systemd/system/${SERVICE_NAME}.service
    systemctl daemon-reload

    # Remove executable
    rm -f "$LINUX_INSTALL_DIR/mcp-ironbase-server"

    echo ""
    echo -e "${GREEN}=== Uninstallation Complete ===${NC}"
    echo ""
    echo -e "${YELLOW}Note: Configuration and data files were preserved:${NC}"
    echo "  Config: $LINUX_CONFIG_DIR/config.toml"
    echo "  Data:   $LINUX_DATA_DIR/data.mlite"
    echo ""
    echo -e "${YELLOW}To completely remove all data:${NC}"
    echo "  sudo rm -rf $LINUX_DATA_DIR $LINUX_CONFIG_DIR $LINUX_LOG_DIR"
}

# Uninstall on macOS
uninstall_macos() {
    echo -e "${CYAN}=== IronBase MCP Server Uninstallation (macOS) ===${NC}"
    echo ""

    # Use built-in uninstall command
    if [[ -f "$MACOS_INSTALL_DIR/mcp-ironbase-server" ]]; then
        "$MACOS_INSTALL_DIR/mcp-ironbase-server" uninstall || true
    fi

    # Stop and unload service
    launchctl stop com.ironbase.server 2>/dev/null || true
    launchctl unload /Library/LaunchDaemons/com.ironbase.server.plist 2>/dev/null || true

    # Remove plist
    rm -f /Library/LaunchDaemons/com.ironbase.server.plist

    # Remove executable
    rm -f "$MACOS_INSTALL_DIR/mcp-ironbase-server"

    echo ""
    echo -e "${GREEN}=== Uninstallation Complete ===${NC}"
    echo ""
    echo -e "${YELLOW}Note: Configuration and data files were preserved:${NC}"
    echo "  Config: $MACOS_CONFIG_DIR/config.toml"
    echo "  Data:   $MACOS_DATA_DIR/data.mlite"
    echo ""
    echo -e "${YELLOW}To completely remove all data:${NC}"
    echo "  sudo rm -rf $MACOS_DATA_DIR $MACOS_CONFIG_DIR $MACOS_LOG_DIR"
}

# Print usage
usage() {
    echo "IronBase MCP Server Installer"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --uninstall    Uninstall the service"
    echo "  --help         Show this help message"
    echo ""
    echo "Examples:"
    echo "  sudo $0              # Install service"
    echo "  sudo $0 --uninstall  # Uninstall service"
}

# Main
main() {
    local action="install"

    while [[ $# -gt 0 ]]; do
        case $1 in
            --uninstall|-u)
                action="uninstall"
                shift
                ;;
            --help|-h)
                usage
                exit 0
                ;;
            *)
                echo -e "${RED}Unknown option: $1${NC}"
                usage
                exit 1
                ;;
        esac
    done

    check_root

    if [[ "$OS" == "unknown" ]]; then
        echo -e "${RED}ERROR: Unsupported operating system${NC}"
        echo "This installer supports Linux and macOS only."
        exit 1
    fi

    if [[ "$action" == "install" ]]; then
        if [[ "$OS" == "linux" ]]; then
            install_linux
        else
            install_macos
        fi
    else
        if [[ "$OS" == "linux" ]]; then
            uninstall_linux
        else
            uninstall_macos
        fi
    fi
}

main "$@"
