import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';

import 'controls.dart';
import 'theme.dart';

/// 屏幕视图
enum ViewMode { spectrum, scope, phase }

/// 视口
enum UiViewport { desktop, mobile }

/// 模块右侧的显示窗类型（复刻参考图）
enum ModuleWindow { lcd, lamp, scope }

/// 一个处理模块（参考图的 SUB 1 / SUB 2 / TONE / STABILISE）
class ModuleChannel extends ChangeNotifier {
  ModuleChannel({
    required this.id,
    required this.title,
    required this.tabTitle,
    required this.accent,
    required this.accentLight,
    required this.window,
    required this.freqRange,
    required this.amountRange,
    required this.labelA,
    required this.labelB,
    required this.aRange,
    required this.bRange,
    required this.aUnit,
    required this.bUnit,
    List<String> lcdLabels = const ['FREQ', 'WIDTH'],
    this.lcdUnit = 'Hz',
    this.lcdText,
    this.amountUnit = 'dB',
    required double freq,
    required double amount,
    required double a,
    required double b,
    bool on = true,
  })  : _freq = freq,
        _amount = amount,
        _a = a,
        _b = b,
        _on = on,
        lcdLabels = List<String>.of(lcdLabels);

  final String id;
  String title;
  /// Short label for the mobile tab bar.
  String tabTitle;
  /// 深色机箱用的强调色
  final Color accent;
  /// 浅色机箱必须换成同色系的压深版本，否则指针与读数会在铝面板上糊掉
  final Color accentLight;
  final ModuleWindow window;
  final NeuRange freqRange;
  final NeuRange amountRange;
  final String labelA;
  final String labelB;
  /// LCD labels/unit/override text so the same hardware face can show
  /// buffer/target or the local source instead of a frequency.
  List<String> lcdLabels;
  String lcdUnit;
  String? lcdText;
  /// Footer facts for the physical card (transport, losses).
  String infoA = '—';
  String infoB = '—';
  /// Per-stream readout for the desktop card.
  double rmsDb = -60;
  String pathText = '—';
  int lostFrames = 0;
  int underruns = 0;
  /// 0 good, 1 fair, 2 poor, 3 unknown/offline.
  int health = 3;
  /// Unit shown below the big knob (dB for gain, ms for a target).
  String amountUnit;
  /// Called when a know/switch is moved so the app can apply the value.
  void Function(double value)? onAmount;
  void Function(double value)? onA;
  void Function(double value)? onB;
  void Function(double value)? onFreq;
  void Function(bool muted)? onMute;
  final NeuRange aRange;
  final NeuRange bRange;
  final String aUnit;
  final String bUnit;

  double _freq;
  double _amount;
  double _a;
  double _b;
  bool _on;
  bool _solo = false;
  bool _mute = false;

  double get freq => _freq;
  double get amount => _amount;
  double get a => _a;
  double get b => _b;
  bool get on => _on;
  bool get solo => _solo;
  bool get mute => _mute;
  bool get audible => _on && !_mute;

  double get amountNorm => amountRange.norm(_amount);

  Color accentFor(MetalPalette p) => p.isLight ? accentLight : accent;

  void setFreq(double v) {
    _set(() => _freq = freqRange.clampAbs(v));
    onFreq?.call(_freq);
  }

  void setAmount(double v) {
    _set(() => _amount = amountRange.clampAbs(v));
    onAmount?.call(_amount);
  }

  void setA(double v) {
    _set(() => _a = aRange.clampAbs(v));
    onA?.call(_a);
  }

  void setB(double v) {
    _set(() => _b = bRange.clampAbs(v));
    onB?.call(_b);
  }

  void toggleOn() => _set(() => _on = !_on);
  void toggleSolo() => _set(() => _solo = !_solo);

  void toggleMute() {
    _set(() => _mute = !_mute);
    onMute?.call(_mute);
  }

  void setTitle(String value) {
    if (title == value) return;
    title = value;
    notifyListeners();
  }

  void setTabTitle(String value) {
    if (tabTitle == value) return;
    tabTitle = value;
    notifyListeners();
  }

  void setReadout(String a, String b, String text, String unit) {
    if (lcdLabels[0] == a &&
        lcdLabels[1] == b &&
        lcdText == text &&
        lcdUnit == unit) {
      return;
    }
    lcdLabels = [a, b];
    lcdText = text;
    lcdUnit = unit;
    notifyListeners();
  }

  void setInfo(String a, String b) {
    if (infoA == a && infoB == b) return;
    infoA = a;
    infoB = b;
    notifyListeners();
  }

  void setFacts({
    required double rms,
    required String path,
    required int lost,
    required int under,
    required int health,
  }) {
    if ((rmsDb - rms).abs() < .01 &&
        pathText == path &&
        lostFrames == lost &&
        underruns == under &&
        this.health == health) {
      return;
    }
    rmsDb = rms;
    pathText = path;
    lostFrames = lost;
    underruns = under;
    this.health = health;
    notifyListeners();
  }

  /// Apply a value arriving from the audio engine without re-notifying it.
  void syncAmount(double v) {
    final next = amountRange.clampAbs(v);
    if ((_amount - next).abs() < 1e-3) return;
    _amount = next;
    notifyListeners();
  }

  void syncMute(bool muted) {
    if (_mute == muted) return;
    _mute = muted;
    notifyListeners();
  }

  void syncOn(bool on) {
    if (_on == on) return;
    _on = on;
    notifyListeners();
  }

  void _set(void Function() apply) {
    apply();
    notifyListeners();
  }
}

/// One rendered EQ curve: per-band level scaled by the stream's channel gain.
class EqCurve {
  const EqCurve({
    required this.name,
    required this.color,
    required this.bands,
    required this.gainDb,
    required this.active,
  });

  final String name;
  final Color color;
  final List<double> bands;
  final double gainDb;
  final bool active;
}

String formatFreq(double hz) => hz >= 10000
    ? '${(hz / 1000).toStringAsFixed(1)}k'
    : hz >= 1000
        ? '${(hz / 1000).toStringAsFixed(2)}k'
        : hz.toStringAsFixed(0);

/// 屏幕显示频带（参考图刻度 10 … 500）
const double kScreenMinHz = 20;
const double kScreenMaxHz = 16000;

double hzToX(double hz) =>
    (math.log(hz / kScreenMinHz) / math.log(kScreenMaxHz / kScreenMinHz)).clamp(0.0, 1.0);

double xToHz(double t) => kScreenMinHz * math.pow(kScreenMaxHz / kScreenMinHz, t.clamp(0.0, 1.0)).toDouble();

class ShaperModel extends ChangeNotifier {
  ShaperModel() {
    for (final c in channels) {
      c.addListener(_forward);
    }
  }

  static const List<String> presets = ['Bass Under Attack', 'Round & Deep', 'Tight Sub', 'Air Room'];

  final List<ModuleChannel> channels = [
    ModuleChannel(
      id: 'stream1',
      title: '远端流 1',
      tabTitle: '流 1',
      accent: const Color(0xFF5BA7FF),
      accentLight: const Color(0xFF1A5EAE),
      window: ModuleWindow.lcd,
      freqRange: const NeuRange(10, 500, defaultValue: 100),
      amountRange: const NeuRange(-60, 12, defaultValue: 0),
      labelA: '目标',
      labelB: '主增益',
      aRange: const NeuRange(10, 500, defaultValue: 100),
      bRange: const NeuRange(-24, 12, defaultValue: 0),
      aUnit: 'ms',
      bUnit: 'dB',
      lcdLabels: const ['缓冲', '目标'],
      lcdUnit: 'ms',
      freq: 100,
      amount: 0,
      a: 100,
      b: 0,
    ),
    ModuleChannel(
      id: 'stream2',
      title: '远端流 2',
      tabTitle: '流 2',
      accent: const Color(0xFFFFA23E),
      accentLight: const Color(0xFFA55A05),
      window: ModuleWindow.lcd,
      freqRange: const NeuRange(10, 500, defaultValue: 100),
      amountRange: const NeuRange(-60, 12, defaultValue: 0),
      labelA: '目标',
      labelB: '主增益',
      aRange: const NeuRange(10, 500, defaultValue: 100),
      bRange: const NeuRange(-24, 12, defaultValue: 0),
      aUnit: 'ms',
      bUnit: 'dB',
      lcdLabels: const ['缓冲', '目标'],
      lcdUnit: 'ms',
      freq: 100,
      amount: 0,
      a: 100,
      b: 0,
    ),
  ];

  ViewMode _view = ViewMode.spectrum;
  UiViewport _viewport = UiViewport.desktop;
  AppThemeMode _theme = AppThemeMode.dark;
  bool _playing = true;
  bool _bypass = false;
  bool _power = true;
  double _input = -12;
  double _output = -6;
  double _hp = 24;
  double _lp = 620;
  double _dry = 0;
  double _wet = 100;
  int _presetIndex = 0;
  int _mobileModule = 0;

  /// Siren integration hooks. The stock model keeps working for previews.
  bool joined = false;
  String statusText = '未组网';
  List<String> peerNames = const [];
  List<EqCurve> eqCurves = const [];
  int streamPage = 0;
  int streamPages = 1;
  // Inline join settings shown on the PC chassis.
  String nodeName = '';
  String sourceKey = 'tone';
  double toneFreq = 440;
  bool outputOn = true;
  double targetMs = 100;
  double masterGainDb = 0;
  bool busy = false;
  String meshRole = '等待';
  String meshLink = '等待';
  int onlinePeers = 0;
  /// Unique mesh devices (discovered peers and stream origins).
  int meshDevices = 1;
  ValueChanged<String>? onNameChanged;
  ValueChanged<String>? onSourceChanged;
  ValueChanged<double>? onToneFreqChanged;
  ValueChanged<bool>? onOutputChanged;
  ValueChanged<double>? onTargetChanged;
  ValueChanged<double>? onMasterChanged;
  VoidCallback? onJoinToggle;
  VoidCallback? onPower;
  VoidCallback? onSettings;
  VoidCallback? onPrev;
  VoidCallback? onNext;
  ValueChanged<double>? onHp;
  ValueChanged<double>? onLp;
  VoidCallback? onBypass;

  static const inputRange = NeuRange(-48, 12, defaultValue: -12);
  static const outputRange = NeuRange(-48, 12, defaultValue: -6);
  static const hpRange = NeuRange(10, 500, defaultValue: 100);
  static const lpRange = NeuRange(-24, 12, defaultValue: 0);
  static const dryWetRange = NeuRange(0, 100, defaultValue: 50);

  ViewMode get view => _view;
  UiViewport get viewport => _viewport;
  AppThemeMode get theme => _theme;
  MetalPalette get palette => MetalPalette.of(_theme);
  bool get playing => _playing;
  bool get bypass => _bypass;
  bool get power => _power && !_bypass;
  double get input => _input;
  double get output => _output;
  double get hp => _hp;
  double get lp => _lp;
  double get dry => _dry;
  double get wet => _wet;
  String get preset => statusText;
  String get sourceLabel => switch (sourceKey) {
        'tone' => '测试音',
        'loopback' => '系统内录',
        'microphone' => '麦克风',
        _ => sourceKey,
      };
  String get joinLabel => busy ? '处理中…' : (joined ? '离开组网' : '加入组网');
  int get mobileModule => _mobileModule;
  bool get anySolo => channels.any((c) => c.solo);

  void setView(ViewMode v) => _model(() => _view = v);
  void setViewport(UiViewport v) => _model(() => _viewport = v);
  void setTheme(AppThemeMode t) => _model(() => _theme = t);
  void toggleTheme() => setTheme(_theme == AppThemeMode.dark ? AppThemeMode.light : AppThemeMode.dark);
  void setMobileModule(int i) => _model(() => _mobileModule = i);
  void togglePlay() => _model(() => _playing = !_playing);
  void toggleBypass() {
    _model(() => _bypass = !_bypass);
    onBypass?.call();
  }
  void togglePower() {
    if (onPower != null) {
      onPower!();
      return;
    }
    _model(() => _power = !_power);
  }

  void syncBypass(bool value) {
    if (_bypass == value) return;
    _bypass = value;
    notifyListeners();
  }

  void setEqCurves(List<EqCurve> curves) {
    eqCurves = curves;
    notifyListeners();
  }

  void setStreamPages(int pages) {
    final next = pages < 1 ? 1 : pages;
    if (streamPages == next) return;
    streamPages = next;
    notifyListeners();
  }

  void setStreamPage(int page) {
    final next = page.clamp(0, streamPages - 1);
    if (streamPage == next) return;
    streamPage = next;
    notifyListeners();
  }

  void setInlineSettings({
    required String name,
    required String source,
    required double tone,
    required bool out,
    required double target,
    required double master,
    required bool isBusy,
    required String role,
    required String link,
    required int peers,
    required int devices,
  }) {
    final changed = nodeName != name ||
        sourceKey != source ||
        (toneFreq - tone).abs() > .01 ||
        outputOn != out ||
        (targetMs - target).abs() > .01 ||
        (masterGainDb - master).abs() > .01 ||
        busy != isBusy ||
        meshRole != role ||
        meshLink != link ||
        onlinePeers != peers ||
        meshDevices != devices;
    nodeName = name;
    sourceKey = source;
    toneFreq = tone;
    outputOn = out;
    targetMs = target;
    masterGainDb = master;
    busy = isBusy;
    meshRole = role;
    meshLink = link;
    onlinePeers = peers;
    meshDevices = devices;
    if (changed) notifyListeners();
  }

  void editNodeName(String value) {
    if (nodeName == value) return;
    nodeName = value;
    onNameChanged?.call(value);
    notifyListeners();
  }

  void selectSource(String value) {
    if (sourceKey == value) return;
    sourceKey = value;
    onSourceChanged?.call(value);
    notifyListeners();
  }

  void editToneFreq(double value) {
    toneFreq = value;
    onToneFreqChanged?.call(value);
    notifyListeners();
  }

  void toggleOutput() {
    outputOn = !outputOn;
    onOutputChanged?.call(outputOn);
    notifyListeners();
  }

  void editTarget(double value) {
    targetMs = value;
    onTargetChanged?.call(value);
    notifyListeners();
  }

  void editMaster(double value) {
    masterGainDb = value;
    onMasterChanged?.call(value);
    notifyListeners();
  }

  void toggleJoin() {
    if (busy) return;
    onJoinToggle?.call();
  }

  void setPeers(List<String> names) {
    if (listEquals(peerNames, names)) return;
    peerNames = names;
    notifyListeners();
  }

  void setJoined(bool value) {
    if (_power == value) return;
    _power = value;
    joined = value;
    notifyListeners();
  }

  void setStatusText(String value) {
    if (statusText == value) return;
    statusText = value;
    notifyListeners();
  }

  void setLevels(double input, double output) {
    var changed = false;
    if ((_input - input).abs() > .05) {
      _input = inputRange.clampAbs(input);
      changed = true;
    }
    if ((_output - output).abs() > .05) {
      _output = outputRange.clampAbs(output);
      changed = true;
    }
    if (changed) notifyListeners();
  }
  void setInput(double v) => _model(() => _input = inputRange.clampAbs(v));
  void setOutput(double v) => _model(() => _output = outputRange.clampAbs(v));
  void setHp(double v) {
    _model(() => _hp = hpRange.clampAbs(v));
    onHp?.call(_hp);
  }

  void setLp(double v) {
    _model(() => _lp = lpRange.clampAbs(v));
    onLp?.call(_lp);
  }

  void setDry(double v) => _model(() {
        _dry = dryWetRange.clampAbs(v);
        _wet = 100 - _dry;
      });

  void setWet(double v) => _model(() {
        _wet = dryWetRange.clampAbs(v);
        _dry = 100 - _wet;
      });

  void nextPreset() {
    if (onNext != null) {
      onNext!();
      return;
    }
    _model(() => _presetIndex = (_presetIndex + 1) % presets.length);
  }

  void prevPreset() {
    if (onPrev != null) {
      onPrev!();
      return;
    }
    _model(() => _presetIndex = (_presetIndex - 1 + presets.length) % presets.length);
  }

  void reset() {
    if (onSettings != null) {
      onSettings!();
      return;
    }
    for (final c in channels) {
      c.setFreq(c.freqRange.defaultValue);
      c.setAmount(c.amountRange.defaultValue);
      c.setA(c.aRange.defaultValue);
      c.setB(c.bRange.defaultValue);
    }
    _model(() {
      _input = inputRange.defaultValue;
      _output = outputRange.defaultValue;
      _hp = hpRange.defaultValue;
      _lp = lpRange.defaultValue;
      _dry = 0;
      _wet = 100;
    });
  }

  void _model(void Function() apply) {
    apply();
    notifyListeners();
  }

  void _forward() => notifyListeners();

  @override
  void dispose() {
    for (final c in channels) {
      c.removeListener(_forward);
    }
    for (final c in channels) {
      c.dispose();
    }
    super.dispose();
  }
}

/// 模块在屏幕上形成的钟形隆起，返回 0..1 的**归一化峰高**，
/// 由绘制侧再乘以绘图区高度，保证顶到量程时也不会画出视口。
double bellAt(ModuleChannel c, double hz) {
  final d = math.log(hz / c.freq) / math.ln2;
  const width = 1.15;
  return c.amountRange.norm(c.amount) * math.exp(-(d * d) / (width * width));
}

double noise01(double x) {
  final a = math.sin(x * 1.7) * .5 + .5;
  final b = math.sin(x * 3.9 + 1.3) * .5 + .5;
  final c = math.sin(x * 8.3 + 2.7) * .5 + .5;
  return a * .55 + b * .3 + c * .15;
}
