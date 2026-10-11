#!/bin/sh
set -eu
project_root=$(CDPATH= cd -- "$(dirname -- "$0")/../.." && pwd)
build_dir=${JIANXIN_NATIVE_TEST_DIR:-/tmp/jianxin-native-tests}
mkdir -p "$build_dir"
swiftc -module-cache-path "$build_dir/module-cache" \
    "$project_root/ios/Jianxin/Models.swift" \
    "$project_root/ios/Jianxin/MIME.swift" \
    "$project_root/ios/Jianxin/TLSSocket.swift" \
    "$project_root/ios/Jianxin/IMAPClient.swift" \
    "$project_root/ios/Jianxin/SMTPClient.swift" \
    "$project_root/ios/Jianxin/ConfigEncryption.swift" \
    "$project_root/ios/Jianxin/RemoteConfig.swift" \
    "$project_root/mobile/tests/swift/ConfigTests.swift" \
    "$project_root/mobile/tests/swift/ConfigDownloadTests.swift" \
    "$project_root/mobile/tests/swift/ProtocolTests.swift" \
    -o "$build_dir/protocol-tests"
"$build_dir/protocol-tests" "$project_root/mobile/tests/fixtures/config.json"
