import 'package:flutter_hbb/common/hq_quality_diagnostics.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('HQ diagnostics update and clear stale fields', () {
    final data = QualityMonitorData();
    data.updateHqDiagnostics({
      'hq_diagnostics_updated': 'true',
      'target_bitrate': '16000',
      'queue_delay': '42',
      'fallback_reason': 'requested h265, using vp9',
      'qos_state': 'Congested',
      'hardware': 'false',
    });
    expect(data.targetBitrate, '16000');
    expect(data.queueDelay, '42');
    expect(data.fallbackReason, isNotEmpty);
    expect(data.qosState, 'Congested');
    expect(data.hardware, 'false');

    data.updateHqDiagnostics({
      'hq_diagnostics_updated': 'true',
      'target_bitrate': '4000',
      'queue_delay': '',
      'fallback_reason': '',
      'qos_state': '',
      'hardware': 'true',
    });
    expect(data.targetBitrate, '4000');
    expect(data.queueDelay, isNull);
    expect(data.fallbackReason, isNull);
    expect(data.qosState, isNull);
    expect(data.hardware, 'true');
    final report = data.buildReport();
    expect(report, contains('Target Bitrate: 4000kb'));
    expect(report, contains('Reason: -'));
    expect(report, isNot(contains('requested h265')));
  });
}
