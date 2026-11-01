import 'package:flutter/material.dart';

/// 亮/暗两套主题的开关
enum AppThemeMode { dark, light }

/// 金属面板材质 token。
/// dark 复刻参考图的深色机箱，light 是同一台设备的铝色机箱；
/// 所有绘制只读 token，不再散写颜色字面量，否则浅色模式必然出现对比度破洞。
class MetalPalette {
  const MetalPalette({
    required this.isLight,
    required this.deskTop,
    required this.deskBottom,
    required this.shellTop,
    required this.shellBottom,
    required this.moduleTop,
    required this.moduleBottom,
    required this.screen,
    required this.lcd,
    required this.lcdInk,
    required this.bevel,
    required this.grooveDark,
    required this.grooveLight,
    required this.inkStrong,
    required this.ink,
    required this.inkFaint,
    required this.cyan,
    required this.orange,
    required this.violet,
    required this.salmon,
    required this.ledOn,
    required this.ledOff,
    required this.knobTop,
    required this.knobMid,
    required this.knobBottom,
    required this.knobRing,
    required this.knobNeck,
    required this.capMid,
    required this.slotFloor,
    required this.faderSlot,
    required this.faderWell,
    required this.wellTint,
    required this.stripTint,
    required this.editBg,
    required this.pillOffTop,
    required this.pillOffBottom,
    required this.barTop,
    required this.barBottom,
    required this.barEdge,
    required this.screenInkBase,
    required this.shadowBase,
    required this.captionRule,
    required this.bezelTop,
    required this.bezelBottom,
    required this.nodeCore,
  });

  final bool isLight;

  /// 桌面托盘与标题栏（径向渐变两端）：这两组自带 alpha，透明部分由
  /// DWM 的 Acrylic 底板补上模糊后的桌面，所以别在别处再乘一次 alpha
  final Color deskTop, deskBottom;
  final Color shellTop, shellBottom;
  /// 凸起金属件
  final Color moduleTop, moduleBottom;
  /// 大屏与小型显示窗的底
  final Color screen, lcd;
  /// 显示窗里的丝印小字
  final Color lcdInk;
  /// 顶边亮线
  final Color bevel;
  final Color grooveDark, grooveLight;
  final Color inkStrong, ink, inkFaint;
  final Color cyan, orange, violet, salmon;
  final Color ledOn, ledOff;
  /// 旋钮圆顶：亮面 → 中间调 → 暗面；外圈与颈部
  final Color knobTop, knobMid, knobBottom, knobRing, knobNeck;
  /// 推子帽的中间调
  final Color capMid;
  /// 滑槽底部
  final Color slotFloor, faderSlot, faderWell;
  /// 分段器与底栏的凹陷底色
  final Color wellTint, stripTint;
  /// 就地输入时的更深底色
  final Color editBg;
  final Color pillOffTop, pillOffBottom;
  /// 屏内锯齿频谱条
  final Color barTop, barBottom, barEdge;
  /// 屏内线条/水印的基色（暗色用白，浅色用墨）
  final Color screenInkBase;
  /// 投影基色
  final Color shadowBase;
  final Color captionRule;
  /// 手机预览外框
  final Color bezelTop, bezelBottom;
  /// 包络节点的亮心
  final Color nodeCore;

  static const MetalPalette dark = MetalPalette(
    isLight: false,
    // 托盘与标题栏自带 alpha：DWM 的 Acrylic 底板会把桌面模糊后垫在下面
    deskTop: Color(0x9E3E434D),
    deskBottom: Color(0x9E22252B),
    shellTop: Color(0xD450555F),
    shellBottom: Color(0xD4363A42),
    moduleTop: Color(0xFF484D57),
    moduleBottom: Color(0xFF35383F),
    screen: Color(0xFF15161A),
    lcd: Color(0xFF0E1014),
    lcdInk: Color(0xFF7C828D),
    bevel: Color(0xFF71767F),
    grooveDark: Color(0xFF1E2025),
    grooveLight: Color(0xFF63686F),
    inkStrong: Color(0xFFEDEFF3),
    ink: Color(0xFFB6BCC6),
    inkFaint: Color(0xFF7C828D),
    cyan: Color(0xFF4FE3F0),
    orange: Color(0xFFFFA23E),
    violet: Color(0xFF7B6CFF),
    salmon: Color(0xFFFF7E63),
    ledOn: Color(0xFF3FD68E),
    ledOff: Color(0xFF33363D),
    knobTop: Color(0xFF5A5F6A),
    knobMid: Color(0xFF3B3F47),
    knobBottom: Color(0xFF23262C),
    knobRing: Color(0xFF2A2D33),
    knobNeck: Color(0xFF26292F),
    capMid: Color(0xFF41454E),
    slotFloor: Color(0xFF1A1C21),
    faderSlot: Color(0xFF141619),
    faderWell: Color(0xFF24272D),
    wellTint: Color(0xFF2A2D34),
    stripTint: Color(0xFF2C2F36),
    editBg: Color(0xFF090A0D),
    pillOffTop: Color(0xFF2C2F36),
    pillOffBottom: Color(0xFF232529),
    barTop: Color(0xFF4C515B),
    barBottom: Color(0xFF2C2F36),
    barEdge: Color(0xFF6A707B),
    screenInkBase: Color(0xFFFFFFFF),
    shadowBase: Color(0xFF000000),
    captionRule: Color(0xFF1B1D21),
    bezelTop: Color(0xFF5A5F6A),
    bezelBottom: Color(0xFF26292F),
    nodeCore: Color(0xFFFFFFFF),
  );

  /// 铝色机箱：面板变亮，屏幕与显示窗改成浅灰，强调色一律压深以保住对比
  static const MetalPalette light = MetalPalette(
    isLight: true,
    deskTop: Color(0x96EAEFF6),
    deskBottom: Color(0x96C2C9D6),
    shellTop: Color(0xD0DFE4EC),
    shellBottom: Color(0xD0BFC6D3),
    moduleTop: Color(0xFFE9EDF4),
    moduleBottom: Color(0xFFC6CCD9),
    screen: Color(0xFFE1E7F0),
    lcd: Color(0xFFCBD3DF),
    lcdInk: Color(0xFF4E5766),
    bevel: Color(0xFFFFFFFF),
    grooveDark: Color(0xFF99A1B1),
    grooveLight: Color(0xFFFFFFFF),
    inkStrong: Color(0xFF171C25),
    ink: Color(0xFF3B4351),
    inkFaint: Color(0xFF69727F),
    cyan: Color(0xFF0B7C8C),
    orange: Color(0xFFB25E06),
    violet: Color(0xFF4B44D0),
    salmon: Color(0xFFCE4A27),
    ledOn: Color(0xFF149258),
    ledOff: Color(0xFFA3AABA),
    knobTop: Color(0xFFFAFBFE),
    knobMid: Color(0xFFD6DBE5),
    knobBottom: Color(0xFFAAB2C1),
    knobRing: Color(0xFF8A92A3),
    knobNeck: Color(0xFFB4BBC8),
    capMid: Color(0xFFCBD1DD),
    slotFloor: Color(0xFFA9B1C0),
    faderSlot: Color(0xFF9CA4B4),
    faderWell: Color(0xFFC9CFDB),
    wellTint: Color(0xFFD2D8E3),
    stripTint: Color(0xFFD5DAE5),
    editBg: Color(0xFFBFC7D4),
    pillOffTop: Color(0xFFCED4DF),
    pillOffBottom: Color(0xFFB8BFCD),
    barTop: Color(0xFFB3BAC8),
    barBottom: Color(0xFFD3D8E2),
    barEdge: Color(0xFF7F8798),
    screenInkBase: Color(0xFF1B2029),
    shadowBase: Color(0xFF414A5A),
    captionRule: Color(0xFFA6ADBC),
    bezelTop: Color(0xFFD8DDE7),
    bezelBottom: Color(0xFFA5ACBA),
    nodeCore: Color(0xFFFFFFFF),
  );

  static MetalPalette of(AppThemeMode mode) => mode == AppThemeMode.dark ? dark : light;

  /// 凹陷/投影：浅色机箱用石板灰而不是纯黑，α 同步压一档，否则会糊成脏块
  Color shade(double alpha) =>
      shadowBase.withValues(alpha: (alpha * (isLight ? .62 : 1)).clamp(0.0, 1.0));

  /// 金属高光
  Color glint(double alpha) =>
      const Color(0xFFFFFFFF).withValues(alpha: (alpha * (isLight ? 1.35 : 1)).clamp(0.0, 1.0));

  /// 屏幕内的线与字：浅色模式底子是墨，同样的 α 会太淡，放大后再用
  Color screenInk(double alpha) =>
      screenInkBase.withValues(alpha: (alpha * (isLight ? 2.1 : 1)).clamp(0.0, 1.0));

  Color get accent => cyan;
  Color get ledDim => ledOff;
}

/// 下凹件：屏幕、LCD、滑槽
class NeuWell extends StatelessWidget {
  const NeuWell({
    super.key,
    this.size,
    required this.radius,
    required this.palette,
    this.child,
    this.padding,
    this.deep = false,
    this.tint,
    this.sheen = false,
  });

  final Size? size;
  final double radius;
  final MetalPalette palette;
  final Widget? child;
  final EdgeInsetsGeometry? padding;
  final bool deep;
  final Color? tint;
  final bool sheen;

  @override
  Widget build(BuildContext context) {
    final r = BorderRadius.circular(radius);
    return SizedBox(
      width: size?.width,
      height: size?.height,
      child: DecoratedBox(
        decoration: BoxDecoration(
          color: tint ?? palette.screen,
          borderRadius: r,
          boxShadow: [
            BoxShadow(color: palette.bevel.withValues(alpha: .5), offset: const Offset(0, 1), blurRadius: 0),
          ],
        ),
        child: Stack(
          fit: StackFit.expand,
          children: [
            if (sheen)
              Positioned.fill(
                child: IgnorePointer(
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      borderRadius: r,
                      gradient: LinearGradient(
                        begin: Alignment.topCenter,
                        end: Alignment.bottomCenter,
                        colors: [palette.glint(.055), Colors.transparent],
                        stops: const [0, .35],
                      ),
                    ),
                  ),
                ),
              ),
            CustomPaint(
              painter: InnerShadowPainter(
                radius: radius,
                shadows: [
                  NeuInnerShadow(
                    offset: Offset(0, deep ? 3 : 2),
                    spread: deep ? 18 : 12,
                    blur: deep ? 9 : 6,
                    color: palette.shade(deep ? .92 : .8),
                  ),
                  NeuInnerShadow(
                    offset: const Offset(0, -1),
                    spread: 4,
                    blur: 2.5,
                    color: palette.bevel.withValues(alpha: .35),
                  ),
                ],
              ),
            ),
            if (padding != null)
              Padding(padding: padding!, child: child)
            else if (child != null)
              child!,
          ],
        ),
      ),
    );
  }
}

class NeuInnerShadow {
  const NeuInnerShadow({
    required this.offset,
    this.spread = 10,
    this.blur = 7,
    required this.color,
  });
  final Offset offset;
  final double spread;
  final double blur;
  final Color color;
}

class InnerShadowPainter extends CustomPainter {
  InnerShadowPainter({required this.radius, required this.shadows});

  final double radius;
  final List<NeuInnerShadow> shadows;

  @override
  void paint(Canvas canvas, Size size) {
    final rect = Offset.zero & size;
    canvas.save();
    canvas.clipRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)));
    final path = Path()..addRRect(RRect.fromRectAndRadius(rect, Radius.circular(radius)));
    for (final s in shadows) {
      canvas.save();
      canvas.translate(s.offset.dx, s.offset.dy);
      canvas.drawPath(
        path,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = s.spread
          ..color = s.color
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, s.blur),
      );
      canvas.restore();
    }
    canvas.restore();
  }

  @override
  bool shouldRepaint(InnerShadowPainter old) => old.radius != radius || old.shadows != shadows;
}

/// 凸起件：金属模块，带顶边高光
class NeuRaised extends StatelessWidget {
  const NeuRaised({
    super.key,
    required this.palette,
    this.radius = 14,
    this.color,
    this.padding,
    this.child,
    this.size,
    this.elevation = 1.0,
    this.pressed = false,
  });

  final MetalPalette palette;
  final double radius;
  final Color? color;
  final EdgeInsetsGeometry? padding;
  final Widget? child;
  final Size? size;
  final double elevation;
  final bool pressed;

  @override
  Widget build(BuildContext context) {
    final r = BorderRadius.circular(radius);
    return SizedBox(
      width: size?.width,
      height: size?.height,
      child: DecoratedBox(
        decoration: BoxDecoration(
          borderRadius: r,
          gradient: LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: pressed
                ? [palette.moduleBottom, palette.moduleBottom]
                : [palette.moduleTop, palette.moduleBottom],
          ),
          boxShadow: [
            BoxShadow(
              color: palette.shade(.55 * elevation),
              offset: Offset(2 * elevation, 4 * elevation),
              blurRadius: 9 * elevation,
            ),
            BoxShadow(
              color: palette.bevel.withValues(alpha: pressed ? .18 : .55),
              offset: Offset(0, -1 * elevation),
              blurRadius: 0,
            ),
          ],
        ),
        child: ClipRRect(
          borderRadius: r,
          child: Stack(
            fit: StackFit.expand,
            children: [
              Positioned.fill(
                child: IgnorePointer(
                  child: DecoratedBox(
                    decoration: BoxDecoration(
                      gradient: LinearGradient(
                        begin: Alignment.topCenter,
                        end: Alignment.bottomCenter,
                        colors: [palette.glint(.06), Colors.transparent],
                        stops: const [0, .3],
                      ),
                    ),
                  ),
                ),
              ),
              if (padding != null)
                Padding(padding: padding!, child: child)
              else if (child != null)
                child!,
            ],
          ),
        ),
      ),
    );
  }
}

/// 字体：深色底上需要更亮一档的字重
class NeuType {
  static const String family = 'Segoe UI';

  static TextStyle chrome(double size, Color color, {FontWeight w = FontWeight.w600, double tracking = 1.2}) =>
      TextStyle(
        fontFamily: family,
        fontSize: size,
        color: color,
        fontWeight: w,
        letterSpacing: tracking,
        height: 1.1,
      );

  static TextStyle body(Color color, {double size = 13, FontWeight w = FontWeight.w500}) => TextStyle(
        fontFamily: family,
        fontSize: size,
        color: color,
        fontWeight: w,
        height: 1.2,
      );

  static TextStyle numeric(double size, Color color, {FontWeight w = FontWeight.w700}) => TextStyle(
        fontFamily: 'Consolas',
        fontFamilyFallback: const ['Cascadia Mono', 'Courier New'],
        fontSize: size,
        color: color,
        fontWeight: w,
        letterSpacing: .5,
        height: 1.0,
      );
}
