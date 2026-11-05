import 'package:flutter/material.dart';

/// 新控制台的设计 token。
///
/// 排版取自参考图 1（宽面板：深色标题条 + 左屏/右格栅 + 读数条 + 单行控件），
/// 控件外形取自参考图 2（白塑面 + 点阵刻度环旋钮 + 帽式推子 + 分段胶囊 + ADV 开关）。
/// 所有绘制只读这里的常量，不在 painter 里散写字面量。
class TantrumTokens {
  const TantrumTokens();

  // ---------------------------------------------------------------- 材质
  /// 桌面渐变两端（参考图 1 的背景实测 202,211,220 → 218,223,229）
  static const Color deskTop = Color(0xFFCAD2DA);
  static const Color deskBottom = Color(0xFFD9DEE6);
  /// 面板白塑面
  static const Color panelTop = Color(0xFFFFFFFF);
  static const Color panelBottom = Color(0xFFF1F3F7);
  static const Color panelShadow = Color(0x5578849A);
  static const Color panelBevel = Color(0x80FFFFFF);
  /// 标题条与示波器屏的深色面
  static const Color headerTop = Color(0xFF474C54);
  static const Color headerBottom = Color(0xFF373C43);
  static const Color screenTop = Color(0xFF454A52);
  static const Color screenBottom = Color(0xFF383D44);
  static const Color screenInk = Color(0xFFE9ECF1);
  static const Color screenGrid = Color(0x2CFFFFFF);
  static const Color screenGridFloor = Color(0x3CFFFFFF);
  /// 凹陷（EQ 图窗）
  static const Color wellBottom = Color(0xFFEDEFF3);
  static const Color wellInk = Color(0xFF8A93A1);
  /// 电平柱与推子槽的底：比面板暗两档，光柱/帽才读得出是陷进面板的
  static const Color meterFloor = Color(0xFFC6CDD7);
  static const Color meterFloorDark = Color(0xFFB4BCC8);

  // ---------------------------------------------------------------- 油墨
  static const Color inkStrong = Color(0xFF2C3138);
  static const Color ink = Color(0xFF5A6270);
  static const Color inkFaint = Color(0xFF9AA3B0);
  static const Color inkOnDark = Color(0xFFE9ECF1);
  static const Color inkOnDarkFaint = Color(0x8AE9ECF1);

  // ---------------------------------------------------------------- 强调
  /// 参考图 1 的数字与旋钮弧是这一支红
  static const Color accent = Color(0xFFE4462C);
  static const Color accentSoft = Color(0x33E4462C);
  /// 参考图 2 的 EQ 曲线
  static const Color curve = Color(0xFFC9773E);

  // ---------------------------------------------------------------- 控件
  static const Color knobBottom = Color(0xFFE9ECF1);
  /// 旋钮受光面渐变的最暗端：比 knobBottom 再压一档，圆柱才立得起来
  static const Color knobShade = Color(0xFFD6DCE5);
  static const Color knobEdge = Color(0x6696A0AF);
  static const Color knobShadow = Color(0x407E8A9C);
  static const Color notch = Color(0xFF2C3138);
  static const Color tickActive = Color(0xFF6E7684);
  static const Color tickIdle = Color(0xFFC2C9D3);
  static const Color capTop = Color(0xFFFFFFFF);
  static const Color capBottom = Color(0xFFE4E8EE);
  static const Color segTrack = Color(0xFFD7DDE5);
  static const Color segOn = Color(0xFFFFFFFF);
  static const Color switchOn = Color(0xFF3A3F45);
  static const Color ledOn = Color(0xFFE4462C);
  static const Color ledOff = Color(0xFFD6DBE2);

  // ---------------------------------------------------------------- 凹凸
  /// 参考图的光从左上打：凸件右下投影、左上回白；凹件左上内阴影、右下内高光。
  static const Color castDark = Color(0x4D9FAAB9);
  static const Color rimLight = Color(0xEFFFFFFF);
  static const Color rimShade = Color(0x408894A4);
  static const Color insetDark = Color(0x6696A2B2);
  static const Color insetLight = Color(0xE6FFFFFF);
  static const Color edgeHairline = Color(0x2E8E9AAC);
  /// 屏与格栅这类大面的受光/背光两端
  static const Color screenLight = Color(0xFF4E545C);
  static const Color screenShade = Color(0xFF2F343A);
  static const Color grilleLight = Color(0xFFFFFFFF);
  static const Color grilleShade = Color(0xFFDFE5ED);

  // ---------------------------------------------------------------- 字体
  static const String family = 'Segoe UI';
  static const double panelRatio = 1.82;

  /// 面板上的丝印小标签：全大写、字距拉开。
  /// 白塑面上的丝印是压印的——下缘回白、上缘留暗，否则字像浮在面板上。
  static TextStyle caption(double s, {Color color = inkFaint, bool onDark = false}) => TextStyle(
        fontFamily: family,
        fontSize: 8.4 * s,
        height: 1.0,
        letterSpacing: 1.05 * s,
        fontWeight: FontWeight.w700,
        decoration: TextDecoration.none,
        color: color,
        shadows: onDark
            ? [Shadow(color: const Color(0x59000000), offset: Offset(0, -0.7 * s))]
            : [Shadow(color: const Color(0xE0FFFFFF), offset: Offset(0, 0.7 * s))],
      );

  /// 控件下方的数值（"50db" 那一类）
  static TextStyle value(double s, {Color color = ink}) => TextStyle(
        fontFamily: family,
        fontSize: 9.6 * s,
        height: 1.0,
        letterSpacing: 0.4 * s,
        fontWeight: FontWeight.w600,
        decoration: TextDecoration.none,
        color: color,
        shadows: [Shadow(color: const Color(0xE0FFFFFF), offset: Offset(0, 0.7 * s))],
      );

  /// 读数条上的数字（红）
  static TextStyle readout(double s) => TextStyle(
        fontFamily: family,
        fontSize: 11.5 * s,
        height: 1.0,
        letterSpacing: 0.2 * s,
        fontWeight: FontWeight.w700,
        decoration: TextDecoration.none,
        color: accent,
        shadows: [Shadow(color: const Color(0xE0FFFFFF), offset: Offset(0, 0.8 * s))],
      );

  /// 标题条上的品牌字
  static TextStyle brand(double s) => TextStyle(
        fontFamily: family,
        fontSize: 12.5 * s,
        height: 1.0,
        letterSpacing: 2.4 * s,
        fontWeight: FontWeight.w700,
        decoration: TextDecoration.none,
        color: inkOnDark,
        shadows: [Shadow(color: const Color(0x80000000), offset: Offset(0, -0.8 * s))],
      );

  /// 标题条上的预设名
  static TextStyle preset(double s) => TextStyle(
        fontFamily: family,
        fontSize: 10 * s,
        height: 1.0,
        letterSpacing: 1.1 * s,
        fontWeight: FontWeight.w600,
        decoration: TextDecoration.none,
        color: inkOnDark,
        shadows: [Shadow(color: const Color(0x80000000), offset: Offset(0, -0.7 * s))],
      );
}

/// 由面板宽度推导出的全部尺寸：面板按参考图 1 的 1.82 宽高比铺开，
/// 内部所有带高、间距、控件直径都是面板宽度的固定比例。
class TantrumMetrics {
  const TantrumMetrics(this.width)
      : scale = width / 455.0,
        height = width / TantrumTokens.panelRatio;

  final double width;
  final double height;

  /// 参考图 1 的像素基准是 455 宽，这里把它归一化成 1.0
  final double scale;
  double get _s => scale;

  double get radius => 14 * _s;
  double get inset => 8 * _s;

  /// 屏/格栅沉进面板的那一圈槽宽：没有它就永远是"贴纸"
  double get well => 3.4 * _s;

  // 纵向带（参考图 1 实测：标题条 y8..34、双屏 45..145、读数 150..190、控件 195..250）
  double get headerTop => 8 * _s;
  double get headerHeight => 26 * _s;
  double get bodyTop => 45 * _s;
  double get bodyHeight => 100 * _s;
  double get readoutTop => 150 * _s;
  double get readoutHeight => 26 * _s;
  double get controlTop => 176 * _s;
  double get controlHeight => 74 * _s;

  /// 左屏 : 右格栅 = 0.50 : 0.374，两侧各留一条电平柱，中间三道缝
  double get _inner => width - 2 * inset;
  double get meterWidth => _inner * 0.045;
  double get meterSlotWidth => meterWidth * 0.46;
  double get meterLabelHeight => 16 * _s;
  double get meterTravel => bodyHeight - meterLabelHeight - 6 * _s;
  double get screenWidth => _inner * 0.50;
  double get grilleWidth => _inner * 0.374;
  double get paneGap => _inner * 0.012;

  /// 四条带（IN | 屏 | 格栅 | OUT）的左边界
  double get meterLeftX => inset;
  double get screenLeftX => inset + meterWidth + paneGap;
  double get grilleLeftX => screenLeftX + screenWidth + paneGap;
  double get meterRightX => width - inset - meterWidth;

  // 旋钮：直径 40，刻度环半径 = 半径 × 1.14，指示槽在 0.86 半径处
  double get knobDiameter => 40 * _s;
  double get knobBoxHeight => knobDiameter + 8 * _s;
  double get knobRingRadius => knobDiameter / 2 * 1.14;
  /// 指示槽必须落在顶面里（顶面半径 = 半径 − 侧壁宽），压在侧壁上会像刻错了位
  double get knobNotchRadius => (knobDiameter / 2 - 4.6 * _s) * 0.66;
  double get knobNotchLength => 9.5 * _s;
  double get knobNotchWidth => 3.4 * _s;
  double get tickCount => 27;
  double get tickRadius => 1.5 * _s;

  /// 旋钮中心横向：参考图 1 实测在面板内 x = 22/96/253/332/410（面板宽 455）。
  /// 首尾各内收 6，避免刻度环被面板圆角裁掉。
  List<double> get knobCenters => [
        28 / 455 * width,
        96 / 455 * width,
        253 / 455 * width,
        332 / 455 * width,
        408 / 455 * width,
      ];

  /// 中间箭头 + 胶囊：落在第 2、3 个旋钮的正中
  double get centerClusterX => 174 / 455 * width;
  double get labelY => controlTop + 4 * _s;
  double get knobCenterY => controlTop + 30 * _s;
  double get valueY => controlTop + 52 * _s;

  // 推子（图2：帽 26×13、槽宽 5、行程 120）
  double get faderTravel => 58 * _s;
  double get faderSlotWidth => 5 * _s;
  double get faderCapWidth => 26 * _s;
  double get faderCapHeight => 13 * _s;
  double get faderPitch => 48 * _s;

  // 分段胶囊（图2：高 26、圆角 13、等宽段）
  double get segHeight => 22 * _s;
  double get segSegmentWidth => 46 * _s;

  /// ADV 开关（图2：56×28，滑块 22）
  double get switchWidth => 44 * _s;
  double get switchHeight => 22 * _s;

  /// 电源/齿轮圆钮
  double get roundButton => 22 * _s;
  double get ledDiameter => 8 * _s;
}
