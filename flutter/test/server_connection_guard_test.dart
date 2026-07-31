import 'package:flutter_hbb/common/server_connection_guard.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('outgoing connection is blocked only for missing/public server', () {
    expect(shouldBlockOutgoingConnectionForServer(true), isTrue);
    expect(shouldBlockOutgoingConnectionForServer(false), isFalse);
  });

  test('missing server confirmation closes prompt and opens settings', () {
    final calls = <String>[];
    confirmMissingServer(
      () => calls.add('close'),
      () => calls.add('settings'),
    );
    expect(calls, ['close', 'settings']);
  });
}
