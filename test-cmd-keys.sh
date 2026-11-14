#!/bin/bash
# Quick test to verify kitty keyboard protocol is enabled in plat

set -e

echo "Building plat with keyboard protocol support..."
cargo build --release 2>&1 | grep -E "Compiling plat|Finished" || true

echo ""
echo "Testing Cmd-key detection in Ghostty..."
echo ""
echo "Instructions:"
echo "1. Open this file: ./target/release/plat test-cmd-keys.sh"
echo "2. In plat, try these Cmd-keys:"
echo ""
echo "   Cmd-S   → Should SAVE (look for file size change or status bar)"
echo "   Cmd-P   → Should open FUZZY FILE PICKER"
echo "   Cmd-F   → Should open FIND dialog"
echo "   Cmd-Z   → Should UNDO if you made edits"
echo "   Cmd-Q   → Should QUIT plat"
echo ""
echo "3. If these work, Ghostty's kitty keyboard protocol is working!"
echo "4. If they don't work, fallback to Ctrl-* keys (Ctrl-S, Ctrl-P, etc.)"
echo ""
echo "Debug: If running in Ghostty with --verbose-logs, check logs for"
echo "       'PushKeyboardEnhancementFlags' messages"
