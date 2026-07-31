import 'package:flutter_hbb/common/hq_video_profile.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('HQ FPS controls preserve min target max ordering', () {
    var fps = HqFpsRange.normalized(60, 30, 20);
    expect((fps.min, fps.target, fps.max), (60, 60, 60));

    fps = fps.withMax(24);
    expect((fps.min, fps.target, fps.max), (24, 24, 24));

    fps = fps.withMin(90);
    expect((fps.min, fps.target, fps.max), (90, 90, 90));

    fps = HqFpsRange.normalized(10, 30, 60).withTarget(120);
    expect((fps.min, fps.target, fps.max), (10, 60, 60));
  });

  test('profile selection enables HQ and the toggle can disable it', () async {
    final calls = <(String, String)>[];
    Future<void> send(String name, String value) async {
      calls.add((name, value));
    }

    await applyHqProfileSelection(send, 'office-clear');
    await setHqSessionEnabled(send, false);

    expect(calls, [
      ('video-profile', 'office-clear'),
      ('enable-hq-video', 'Y'),
      ('enable-hq-video', 'N'),
    ]);
  });
}
