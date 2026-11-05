import 'dart:math' as math;

import 'package:flutter/foundation.dart';

/// 新控制台的占位数据。**不接引擎**：所有值都活在这里，接线时把
/// 每个字段的 getter/setter 换成 SirenBinding 的真实读写即可。
class TantrumModel extends ChangeNotifier {
  /// 0 = 电平（示波窗），1 = 均衡（EQ 图窗）
  int screenMode = 0;

  /// ADV：右格栅 ↔ 四推子
  bool advanced = false;
  bool powerOn = true;

  /// 归一化旋钮值（0–1），显示时按各自量程映射
  double bufferTarget = 0.189; // 30–400 ms
  double frameLength = 1 / 3; // 2.5 / 5 / 10 / 20 ms
  double mixGain = 2 / 3; // −24…+12 dB
  double outputGain = 0.875; // −48…0 dB
  double sendGain = 1.0; // −48…0 dB

  /// 中间那颗胶囊：true = 混音，false = 直通
  bool mixing = true;

  /// 左右两条电平柱（0–1，对应 −48…0 dBFS）。接线前是占位读数。
  double inLevel = 0.75;
  double outLevel = 0.62;

  /// 预设名（占位）
  String preset = 'MELTDOWN';
  int presetIndex = 1;
  static const List<String> presets = ['MELTDOWN', 'AIR', 'GLASS', 'TUBE'];

  /// 推子（低频 / 中频 / 高频 / 空气）
  final List<double> bands = [0.62, 0.48, 0.36, 0.55];

  // ---------------------------------------------------------------- 格式化

  String get bufferTargetText => '${(30 + bufferTarget * 370).round()}ms';

  String get frameLengthText {
    const steps = ['2.5', '5', '10', '20'];
    return '${steps[(frameLength * 3.999).round()]}ms';
  }

  String get mixGainText {
    final db = (-24 + mixGain * 36);
    return '${db >= 0 ? '+' : ''}${db.round()}db';
  }

  String _negativeOnly(double v) => '${(v * 48 - 48).round()}db';

  String get outputGainText => _negativeOnly(outputGain);
  String get sendGainText => _negativeOnly(sendGain);

  /// 电平柱下方那串 dBFS（图1 的 "-12.0" / "-6.0"）
  static String dbText(double level) => (-48 + level * 48).toStringAsFixed(1);

  String get inText => dbText(inLevel);
  String get outText => dbText(outLevel);

  /// 读数条（占位数字，接线时换成快照值）
  List<TantrumStat> get stats => [
        TantrumStat('节点', '3'),
        TantrumStat('RTT ms', '7'),
        TantrumStat('水位 ms', '98'),
        TantrumStat('丢包', '0.4%'),
      ];

  /// 示波窗波形：一个半周期的方波感（占位）
  List<double> get waveform {
    final out = <double>[];
    for (var i = 0; i < 160; i++) {
      final t = i / 160 * 6 * 2 * math.pi;
      final env = 0.72 + 0.2 * math.sin(i / 160 * 2 * math.pi);
      out.add(math.sin(t) > 0 ? env : -env * 0.92);
    }
    return out;
  }

  /// EQ 图窗的频谱灰区与曲线（占位）
  List<double> get spectrum {
    final out = <double>[];
    for (var i = 0; i < 64; i++) {
      final t = i / 64;
      out.add(((1 - t) * 0.72 + 0.1 * math.sin(t * 22) + 0.06 * math.sin(t * 7)).clamp(0.04, 1.0));
    }
    return out;
  }

  List<double> get eqCurve {
    final out = <double>[];
    for (var i = 0; i < 40; i++) {
      final t = i / 39;
      final shelf = (bands[0] - 0.5) * 2 * math.exp(-t * 6);
      final bell = (bands[1] - 0.5) * 1.4 * math.exp(-math.pow((t - 0.35) * 5, 2));
      final high = (bands[2] - 0.5) * 1.8 * math.max(0, t - 0.55) * 2.2;
      final air = (bands[3] - 0.5) * 1.2 * math.max(0, t - 0.78) * 4;
      out.add((shelf + bell + high + air).clamp(-1.0, 1.0));
    }
    return out;
  }

  // ---------------------------------------------------------------- 交互

  void setBufferTarget(double v) => _set(() => bufferTarget = v);
  void setFrameLength(double v) => _set(() => frameLength = v);
  void setMixGain(double v) => _set(() => mixGain = v);
  void setOutputGain(double v) => _set(() => outputGain = v);
  void setSendGain(double v) => _set(() => sendGain = v);
  void setBand(int index, double v) => _set(() => bands[index] = v);
  void setScreenMode(int mode) => _set(() => screenMode = mode);
  void setAdvanced(bool on) => _set(() => advanced = on);
  void setPower(bool on) => _set(() => powerOn = on);
  void setMixing(bool on) => _set(() => mixing = on);

  void stepPreset(int delta) => _set(() {
        presetIndex = (presetIndex + delta) % presets.length;
        preset = presets[presetIndex];
      });

  void _set(void Function() apply) {
    apply();
    notifyListeners();
  }
}

class TantrumStat {
  const TantrumStat(this.label, this.value);
  final String label;
  final String value;
}
