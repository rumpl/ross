codesign --entitlements /tmp/entitlements.plist --force -s - target/release/ross-daemon
rm -rf /tmp/ross
DYLD_LIBRARY_PATH=/opt/homebrew/lib:$(brew --prefix llvm)/lib ./target/release/ross-daemon start
