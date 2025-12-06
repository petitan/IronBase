#!/bin/bash
# IronBase MCP Server Linux/macOS Installer
#
# Usage:
#   curl -fsSL https://github.com/petitan/IronBase/releases/latest/download/install.sh | sudo bash
#   sudo ./install.sh              # Install (auto-downloads if needed)
#   sudo ./install.sh --update     # Update running service to latest version
#   sudo ./install.sh --uninstall  # Uninstall service
#   sudo ./install.sh --no-download # Install without auto-download

set -e

# Colors
RED='\033[0;31m'
GREEN='\033[0;32m'
YELLOW='\033[1;33m'
CYAN='\033[0;36m'
GRAY='\033[0;90m'
NC='\033[0m' # No Color

# Configuration
SERVICE_NAME="ironbase"
DISPLAY_NAME="IronBase MCP Server"
EXE_NAME="mcp-ironbase-server"

# GitHub Release Configuration
GITHUB_REPO="petitan/IronBase"
GITHUB_API_URL="https://api.github.com/repos/$GITHUB_REPO/releases/latest"

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

# Set paths based on OS
if [[ "$OS" == "linux" ]]; then
    INSTALL_DIR="$LINUX_INSTALL_DIR"
    DATA_DIR="$LINUX_DATA_DIR"
    CONFIG_DIR="$LINUX_CONFIG_DIR"
    LOG_DIR="$LINUX_LOG_DIR"
else
    INSTALL_DIR="$MACOS_INSTALL_DIR"
    DATA_DIR="$MACOS_DATA_DIR"
    CONFIG_DIR="$MACOS_CONFIG_DIR"
    LOG_DIR="$MACOS_LOG_DIR"
fi

# Check root
check_root() {
    if [[ $EUID -ne 0 ]]; then
        echo -e "${RED}ERROR: This script must be run as root (use sudo)${NC}"
        exit 1
    fi
}

# Get architecture for download
get_arch() {
    local arch=$(uname -m)
    case $arch in
        x86_64|amd64)
            echo "x86_64"
            ;;
        aarch64|arm64)
            echo "aarch64"
            ;;
        *)
            echo "$arch"
            ;;
    esac
}

# Download latest release from GitHub
download_latest_release() {
    local dest_path="$1"
    local arch=$(get_arch)
    local os_name=""

    if [[ "$OS" == "linux" ]]; then
        os_name="linux"
    else
        os_name="darwin"
    fi

    echo -e "${CYAN}Checking for latest release from GitHub...${NC}"

    # Get release info
    local release_info
    release_info=$(curl -s -H "Accept: application/vnd.github.v3+json" \
                        -H "User-Agent: IronBase-Installer" \
                        "$GITHUB_API_URL" 2>/dev/null) || {
        echo -e "${RED}Failed to fetch release info${NC}"
        return 1
    }

    # Extract version
    local version
    version=$(echo "$release_info" | grep -o '"tag_name": "[^"]*"' | cut -d'"' -f4)
    echo -e "${GREEN}Latest version: $version${NC}"

    # Find matching asset (try multiple patterns)
    local download_url=""
    local patterns=(
        "${os_name}.*${arch}"
        "${os_name}"
        "${EXE_NAME}"
    )

    for pattern in "${patterns[@]}"; do
        download_url=$(echo "$release_info" | grep -o '"browser_download_url": "[^"]*'"$pattern"'[^"]*"' | head -1 | cut -d'"' -f4)
        if [[ -n "$download_url" ]]; then
            break
        fi
    done

    if [[ -z "$download_url" ]]; then
        echo -e "${YELLOW}No pre-built binary found for $os_name-$arch${NC}"
        echo -e "${YELLOW}Please build from source: cargo build --release${NC}"
        return 1
    fi

    local filename=$(basename "$download_url")
    echo -e "Downloading $filename..."

    curl -L -o "$dest_path" "$download_url" 2>/dev/null || {
        echo -e "${RED}Download failed${NC}"
        return 1
    }

    chmod +x "$dest_path"
    echo -e "${GREEN}Downloaded successfully!${NC}"
    return 0
}

# Find executable locally
find_executable() {
    local script_dir="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

    local paths=(
        "$script_dir/../target/release/$EXE_NAME"
        "./target/release/$EXE_NAME"
        "../target/release/$EXE_NAME"
        "./$EXE_NAME"
        "$script_dir/$EXE_NAME"
    )

    for path in "${paths[@]}"; do
        if [[ -f "$path" ]]; then
            echo "$(cd "$(dirname "$path")" && pwd)/$(basename "$path")"
            return 0
        fi
    done

    return 1
}

# Get current installed version
get_installed_version() {
    local exe_path="$INSTALL_DIR/$EXE_NAME"
    if [[ -f "$exe_path" ]]; then
        "$exe_path" --version 2>/dev/null || echo "unknown"
    else
        echo "not installed"
    fi
}

# Create default config with all options
create_config() {
    local config_path="$1"
    local db_path="$2"

    cat > "$config_path" << EOF
# IronBase MCP Server Configuration

[server]
host = "0.0.0.0"
port = 8080
# Max request body size (supports: B, KB, MB, GB)
# Default: 1GB - suitable for batch operations with large attachments
max_body_size = "1GB"

[database]
path = "$db_path"

[logging]
level = "info"
EOF
}

# Print Claude Desktop config hint
print_claude_hint() {
    local exe_path="$1"

    echo -e "${GRAY}Claude Desktop config (~/.config/Claude/claude_desktop_config.json):${NC}"
    echo -e "${GRAY}{${NC}"
    echo -e "${GRAY}  \"mcpServers\": {${NC}"
    echo -e "${GRAY}    \"ironbase\": {${NC}"
    echo -e "${GRAY}      \"command\": \"$exe_path\",${NC}"
    echo -e "${GRAY}      \"args\": [\"--stdio\"]${NC}"
    echo -e "${GRAY}    }${NC}"
    echo -e "${GRAY}  }${NC}"
    echo -e "${GRAY}}${NC}"
}

# Install on Linux (systemd)
install_linux() {
    local no_download="$1"

    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  IronBase MCP Server Installation${NC}"
    echo -e "${CYAN}  Platform: Linux (systemd)${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""

    local exe_path
    exe_path=$(find_executable) || exe_path=""

    if [[ -z "$exe_path" ]]; then
        if [[ "$no_download" == "true" ]]; then
            echo -e "${RED}ERROR: Cannot find $EXE_NAME executable${NC}"
            echo -e "${YELLOW}Please build the project first: cargo build --release${NC}"
            exit 1
        fi

        echo -e "${YELLOW}Executable not found locally, downloading from GitHub...${NC}"
        local temp_dir=$(mktemp -d)
        exe_path="$temp_dir/$EXE_NAME"

        if ! download_latest_release "$exe_path"; then
            rm -rf "$temp_dir"
            exit 1
        fi
    fi

    echo -e "${GREEN}Source executable: $exe_path${NC}"

    # Create ironbase user if not exists
    if ! id -u ironbase >/dev/null 2>&1; then
        echo "Creating ironbase user..."
        useradd -r -s /bin/false -d "$DATA_DIR" ironbase || true
    fi

    # Create directories
    echo "Creating directories..."
    mkdir -p "$INSTALL_DIR"
    mkdir -p "$DATA_DIR"
    mkdir -p "$CONFIG_DIR"
    mkdir -p "$LOG_DIR"

    # Copy executable
    echo "Installing executable..."
    cp "$exe_path" "$INSTALL_DIR/$EXE_NAME"
    chmod 755 "$INSTALL_DIR/$EXE_NAME"

    # Create default config
    local config_path="$CONFIG_DIR/config.toml"
    if [[ ! -f "$config_path" ]]; then
        echo "Creating default configuration..."
        create_config "$config_path" "$DATA_DIR/data.mlite"
    fi

    # Set ownership
    chown -R ironbase:ironbase "$DATA_DIR"
    chown -R ironbase:ironbase "$LOG_DIR"

    # Install systemd service using built-in command
    echo "Installing systemd service..."
    "$INSTALL_DIR/$EXE_NAME" install 2>/dev/null || {
        echo -e "${YELLOW}Note: Built-in service installer returned non-zero${NC}"
    }

    # Get installed version
    local version
    version=$("$INSTALL_DIR/$EXE_NAME" --version 2>/dev/null || echo "unknown")

    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Installation Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo -e "Version:      ${version}"
    echo -e "Service Name: ${SERVICE_NAME}"
    echo -e "Install Dir:  ${INSTALL_DIR}"
    echo -e "Data Dir:     ${DATA_DIR}"
    echo -e "Config File:  ${config_path}"
    echo -e "Log Dir:      ${LOG_DIR}"
    echo ""
    echo -e "${CYAN}Service Commands:${NC}"
    echo "  Start:   sudo systemctl start $SERVICE_NAME"
    echo "  Stop:    sudo systemctl stop $SERVICE_NAME"
    echo "  Status:  sudo systemctl status $SERVICE_NAME"
    echo "  Logs:    sudo journalctl -u $SERVICE_NAME -f"
    echo ""
    echo -e "${CYAN}Or use built-in commands:${NC}"
    echo "  $EXE_NAME start"
    echo "  $EXE_NAME stop"
    echo "  $EXE_NAME status"
    echo ""
    echo -e "${CYAN}For Claude Desktop (stdio mode):${NC}"
    echo "  $INSTALL_DIR/$EXE_NAME --stdio"
    echo ""
    print_claude_hint "$INSTALL_DIR/$EXE_NAME"

    # Cleanup temp dir if used
    if [[ "$exe_path" == /tmp/* ]]; then
        rm -rf "$(dirname "$exe_path")"
    fi
}

# Install on macOS (launchd)
install_macos() {
    local no_download="$1"

    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  IronBase MCP Server Installation${NC}"
    echo -e "${CYAN}  Platform: macOS (launchd)${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""

    local exe_path
    exe_path=$(find_executable) || exe_path=""

    if [[ -z "$exe_path" ]]; then
        if [[ "$no_download" == "true" ]]; then
            echo -e "${RED}ERROR: Cannot find $EXE_NAME executable${NC}"
            echo -e "${YELLOW}Please build the project first: cargo build --release${NC}"
            exit 1
        fi

        echo -e "${YELLOW}Executable not found locally, downloading from GitHub...${NC}"
        local temp_dir=$(mktemp -d)
        exe_path="$temp_dir/$EXE_NAME"

        if ! download_latest_release "$exe_path"; then
            rm -rf "$temp_dir"
            exit 1
        fi
    fi

    echo -e "${GREEN}Source executable: $exe_path${NC}"

    # Create _ironbase user if not exists (macOS style)
    if ! dscl . -read /Users/_ironbase >/dev/null 2>&1; then
        echo "Creating _ironbase user..."
        # Find available UID
        local uid=400
        while dscl . -list /Users UniqueID | grep -q "\\b$uid\\b"; do
            uid=$((uid + 1))
        done

        dscl . -create /Users/_ironbase 2>/dev/null || true
        dscl . -create /Users/_ironbase UserShell /usr/bin/false 2>/dev/null || true
        dscl . -create /Users/_ironbase UniqueID $uid 2>/dev/null || true
        dscl . -create /Users/_ironbase PrimaryGroupID 20 2>/dev/null || true
        dscl . -create /Users/_ironbase NFSHomeDirectory "$DATA_DIR" 2>/dev/null || true
    fi

    # Create directories
    echo "Creating directories..."
    mkdir -p "$INSTALL_DIR"
    mkdir -p "$DATA_DIR"
    mkdir -p "$CONFIG_DIR"
    mkdir -p "$LOG_DIR"

    # Copy executable
    echo "Installing executable..."
    cp "$exe_path" "$INSTALL_DIR/$EXE_NAME"
    chmod 755 "$INSTALL_DIR/$EXE_NAME"

    # Create default config
    local config_path="$CONFIG_DIR/config.toml"
    if [[ ! -f "$config_path" ]]; then
        echo "Creating default configuration..."
        create_config "$config_path" "$DATA_DIR/data.mlite"
    fi

    # Set ownership
    chown -R _ironbase:staff "$DATA_DIR" 2>/dev/null || chown -R $(whoami) "$DATA_DIR"
    chown -R _ironbase:staff "$LOG_DIR" 2>/dev/null || chown -R $(whoami) "$LOG_DIR"

    # Install launchd service using built-in command
    echo "Installing launchd service..."
    "$INSTALL_DIR/$EXE_NAME" install 2>/dev/null || {
        echo -e "${YELLOW}Note: Built-in service installer returned non-zero${NC}"
    }

    # Get installed version
    local version
    version=$("$INSTALL_DIR/$EXE_NAME" --version 2>/dev/null || echo "unknown")

    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Installation Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo -e "Version:      ${version}"
    echo -e "Service Name: com.ironbase.server"
    echo -e "Install Dir:  ${INSTALL_DIR}"
    echo -e "Data Dir:     ${DATA_DIR}"
    echo -e "Config File:  ${config_path}"
    echo -e "Log Dir:      ${LOG_DIR}"
    echo ""
    echo -e "${CYAN}Service Commands:${NC}"
    echo "  Start:   sudo launchctl start com.ironbase.server"
    echo "  Stop:    sudo launchctl stop com.ironbase.server"
    echo "  Status:  sudo launchctl list | grep ironbase"
    echo "  Logs:    tail -f $LOG_DIR/*.log"
    echo ""
    echo -e "${CYAN}Or use built-in commands:${NC}"
    echo "  $EXE_NAME start"
    echo "  $EXE_NAME stop"
    echo "  $EXE_NAME status"
    echo ""
    echo -e "${CYAN}For Claude Desktop (stdio mode):${NC}"
    echo "  $INSTALL_DIR/$EXE_NAME --stdio"
    echo ""
    print_claude_hint "$INSTALL_DIR/$EXE_NAME"

    # Cleanup temp dir if used
    if [[ "$exe_path" == /tmp/* ]]; then
        rm -rf "$(dirname "$exe_path")"
    fi
}

# Update installation
update_service() {
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  IronBase MCP Server Update${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""

    local exe_path="$INSTALL_DIR/$EXE_NAME"

    # Check if installed
    if [[ ! -f "$exe_path" ]]; then
        echo -e "${YELLOW}IronBase not installed at $INSTALL_DIR${NC}"
        echo -e "Running full installation instead..."
        echo ""
        if [[ "$OS" == "linux" ]]; then
            install_linux "false"
        else
            install_macos "false"
        fi
        return
    fi

    # Get current version
    local current_version
    current_version=$("$exe_path" --version 2>/dev/null || echo "unknown")
    echo -e "Current version: ${current_version}"

    # Check if service is running
    local service_running="false"
    if [[ "$OS" == "linux" ]]; then
        if systemctl is-active --quiet "$SERVICE_NAME" 2>/dev/null; then
            service_running="true"
        fi
    else
        if launchctl list 2>/dev/null | grep -q "com.ironbase.server"; then
            service_running="true"
        fi
    fi

    if [[ "$service_running" == "true" ]]; then
        echo -e "${YELLOW}Stopping service...${NC}"
        if [[ "$OS" == "linux" ]]; then
            systemctl stop "$SERVICE_NAME"
        else
            launchctl stop com.ironbase.server 2>/dev/null || true
        fi
        sleep 2
    fi

    # Backup current executable
    local backup_path="${exe_path}.backup"
    echo "Backing up current version..."
    cp "$exe_path" "$backup_path"

    # Download latest
    echo ""
    local temp_dir=$(mktemp -d)
    local temp_exe="$temp_dir/$EXE_NAME"

    if ! download_latest_release "$temp_exe"; then
        echo -e "${RED}ERROR: Failed to download latest version${NC}"
        rm -rf "$temp_dir"

        if [[ "$service_running" == "true" ]]; then
            echo -e "${YELLOW}Restarting service with old version...${NC}"
            if [[ "$OS" == "linux" ]]; then
                systemctl start "$SERVICE_NAME"
            else
                launchctl start com.ironbase.server
            fi
        fi
        exit 1
    fi

    # Install new version
    echo "Installing new version..."
    cp "$temp_exe" "$exe_path"
    chmod 755 "$exe_path"

    # Get new version
    local new_version
    new_version=$("$exe_path" --version 2>/dev/null || echo "unknown")

    # Restart service if it was running
    if [[ "$service_running" == "true" ]]; then
        echo -e "${GREEN}Starting service...${NC}"
        if [[ "$OS" == "linux" ]]; then
            systemctl start "$SERVICE_NAME"
            sleep 1
            if systemctl is-active --quiet "$SERVICE_NAME"; then
                echo -e "${GREEN}Service started successfully!${NC}"
            else
                echo -e "${RED}WARNING: Service failed to start!${NC}"
                echo -e "${YELLOW}Restoring backup...${NC}"
                cp "$backup_path" "$exe_path"
                systemctl start "$SERVICE_NAME"
            fi
        else
            launchctl start com.ironbase.server
            sleep 1
            if launchctl list 2>/dev/null | grep -q "com.ironbase.server"; then
                echo -e "${GREEN}Service started successfully!${NC}"
            else
                echo -e "${RED}WARNING: Service failed to start!${NC}"
                echo -e "${YELLOW}Restoring backup...${NC}"
                cp "$backup_path" "$exe_path"
                launchctl start com.ironbase.server
            fi
        fi
    fi

    # Cleanup
    rm -rf "$temp_dir"
    rm -f "$backup_path"

    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Update Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo -e "Previous: ${current_version}"
    echo -e "Current:  ${GREEN}${new_version}${NC}"
    echo ""
}

# Uninstall on Linux
uninstall_linux() {
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  IronBase MCP Server Uninstallation${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""

    local exe_path="$INSTALL_DIR/$EXE_NAME"

    # Use built-in uninstall command
    if [[ -f "$exe_path" ]]; then
        echo "Running built-in uninstaller..."
        "$exe_path" uninstall 2>/dev/null || true
    fi

    # Stop and disable service
    echo "Stopping service..."
    systemctl stop $SERVICE_NAME 2>/dev/null || true
    systemctl disable $SERVICE_NAME 2>/dev/null || true

    # Remove service file
    rm -f /etc/systemd/system/${SERVICE_NAME}.service
    systemctl daemon-reload

    # Remove executable
    echo "Removing executable..."
    rm -f "$exe_path"

    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Uninstallation Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo -e "${YELLOW}Data preserved at:${NC}"
    echo "  Config: $CONFIG_DIR/config.toml"
    echo "  Data:   $DATA_DIR/data.mlite"
    echo ""
    echo -e "${GRAY}To remove all data:${NC}"
    echo "  sudo rm -rf $DATA_DIR $CONFIG_DIR $LOG_DIR"
}

# Uninstall on macOS
uninstall_macos() {
    echo -e "${CYAN}========================================${NC}"
    echo -e "${CYAN}  IronBase MCP Server Uninstallation${NC}"
    echo -e "${CYAN}========================================${NC}"
    echo ""

    local exe_path="$INSTALL_DIR/$EXE_NAME"

    # Use built-in uninstall command
    if [[ -f "$exe_path" ]]; then
        echo "Running built-in uninstaller..."
        "$exe_path" uninstall 2>/dev/null || true
    fi

    # Stop and unload service
    echo "Stopping service..."
    launchctl stop com.ironbase.server 2>/dev/null || true
    launchctl unload /Library/LaunchDaemons/com.ironbase.server.plist 2>/dev/null || true

    # Remove plist
    rm -f /Library/LaunchDaemons/com.ironbase.server.plist

    # Remove executable
    echo "Removing executable..."
    rm -f "$exe_path"

    echo ""
    echo -e "${GREEN}========================================${NC}"
    echo -e "${GREEN}  Uninstallation Complete!${NC}"
    echo -e "${GREEN}========================================${NC}"
    echo ""
    echo -e "${YELLOW}Data preserved at:${NC}"
    echo "  Config: $CONFIG_DIR/config.toml"
    echo "  Data:   $DATA_DIR/data.mlite"
    echo ""
    echo -e "${GRAY}To remove all data:${NC}"
    echo "  sudo rm -rf $DATA_DIR $CONFIG_DIR $LOG_DIR"
}

# Print usage
usage() {
    echo "IronBase MCP Server Installer"
    echo ""
    echo "Usage: $0 [OPTIONS]"
    echo ""
    echo "Options:"
    echo "  --update       Update existing installation to latest version"
    echo "  --uninstall    Uninstall the service"
    echo "  --no-download  Don't auto-download if executable not found"
    echo "  --help         Show this help message"
    echo ""
    echo "Examples:"
    echo "  sudo $0                 # Install (auto-downloads if needed)"
    echo "  sudo $0 --update        # Update to latest version"
    echo "  sudo $0 --uninstall     # Uninstall service"
    echo "  sudo $0 --no-download   # Install without auto-download"
    echo ""
    echo "One-liner install:"
    echo "  curl -fsSL https://github.com/$GITHUB_REPO/releases/latest/download/install.sh | sudo bash"
}

# Main
main() {
    local action="install"
    local no_download="false"

    while [[ $# -gt 0 ]]; do
        case $1 in
            --update|-u)
                action="update"
                shift
                ;;
            --uninstall)
                action="uninstall"
                shift
                ;;
            --no-download)
                no_download="true"
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

    case "$action" in
        install)
            if [[ "$OS" == "linux" ]]; then
                install_linux "$no_download"
            else
                install_macos "$no_download"
            fi
            ;;
        update)
            update_service
            ;;
        uninstall)
            if [[ "$OS" == "linux" ]]; then
                uninstall_linux
            else
                uninstall_macos
            fi
            ;;
    esac
}

main "$@"
