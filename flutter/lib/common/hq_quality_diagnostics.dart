class QualityMonitorData {
  String? speed;
  String? fps;
  String? delay;
  String? targetBitrate;
  String? codecFormat;
  String? chroma;
  String? queueDelay;
  String? fallbackReason;
  String? qosState;
  String? hardware;

  void updateHqDiagnostics(Map<String, dynamic> event) {
    if (event['hq_diagnostics_updated'] != 'true') {
      return;
    }
    String? value(String key) {
      final text = event[key]?.toString() ?? '';
      return text.isEmpty ? null : text;
    }

    targetBitrate = value('target_bitrate');
    queueDelay = value('queue_delay');
    fallbackReason = value('fallback_reason');
    qosState = value('qos_state');
    hardware = value('hardware');
  }

  String buildReport() {
    return [
      'Speed: ${speed ?? '-'}',
      'FPS: ${fps ?? '-'}',
      'Delay: ${delay ?? '-'}ms',
      'Target Bitrate: ${targetBitrate ?? '-'}kb',
      'Codec: ${codecFormat ?? '-'}',
      'Chroma: ${chroma ?? '-'}',
      'Queue: ${queueDelay ?? '-'}ms',
      'QoS: ${qosState ?? '-'}',
      'Hardware: ${hardware ?? '-'}',
      'Reason: ${fallbackReason ?? '-'}',
    ].join('\n');
  }
}
