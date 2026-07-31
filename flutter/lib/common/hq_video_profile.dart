class HqFpsRange {
  final int min;
  final int target;
  final int max;

  const HqFpsRange._(this.min, this.target, this.max);

  factory HqFpsRange.normalized(int min, int target, int max) {
    final normalizedMin = min.clamp(1, 120).toInt();
    final normalizedTarget = target.clamp(normalizedMin, 120).toInt();
    final normalizedMax = max.clamp(normalizedTarget, 120).toInt();
    return HqFpsRange._(normalizedMin, normalizedTarget, normalizedMax);
  }

  HqFpsRange withMin(int value) {
    final nextMin = value.clamp(1, 120).toInt();
    return HqFpsRange.normalized(
      nextMin,
      target < nextMin ? nextMin : target,
      max < nextMin ? nextMin : max,
    );
  }

  HqFpsRange withTarget(int value) {
    return HqFpsRange.normalized(min, value.clamp(min, max).toInt(), max);
  }

  HqFpsRange withMax(int value) {
    final nextMax = value.clamp(1, 120).toInt();
    return HqFpsRange.normalized(
      min > nextMax ? nextMax : min,
      target > nextMax ? nextMax : target,
      nextMax,
    );
  }
}

typedef HqSessionOptionSender = Future<void> Function(
    String name, String value);

Future<void> applyHqProfileSelection(
    HqSessionOptionSender sendOption, String profile) async {
  await sendOption('video-profile', profile);
  await sendOption('enable-hq-video', 'Y');
}

Future<void> setHqSessionEnabled(
    HqSessionOptionSender sendOption, bool enabled) {
  return sendOption('enable-hq-video', enabled ? 'Y' : 'N');
}
