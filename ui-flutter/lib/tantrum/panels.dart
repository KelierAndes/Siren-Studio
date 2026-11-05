import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'controls.dart';
import 'relief.dart';
import 'tokens.dart';

/// 参考图 1 的三块面板：深色示波窗、同心点阵格栅、深色标题条。

/// 左屏：网格 + 透视线 + 波形（Siren 语义 = 主输出电平/波形）
class TantrumScope extends StatelessWidget {
  const TantrumScope({
    super.key,
    required this.m,
    required this.waveform,
    required this.title,
    required this.subtitle,
  });

  final TantrumMetrics m;
  final List<double> waveform;
  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      painter: _ScopePainter(m, waveform),
      // CustomPaint 默认把 child 居中，屏幕上的丝印会飘到波形中间
      child: Align(
        alignment: Alignment.topLeft,
        child: Padding(
          padding: EdgeInsets.only(left: 9 * m.scale + m.well, top: 7 * m.scale + m.well),
          child: Row(
            children: [
              Text(title,
                  style: TantrumTokens.caption(m.scale,
                      color: TantrumTokens.inkOnDarkFaint, onDark: true)),
              SizedBox(width: 6 * m.scale),
              Text(subtitle,
                  style: TantrumTokens.caption(m.scale,
                      color: TantrumTokens.inkOnDark, onDark: true)),
            ],
          ),
        ),
      ),
    );
  }
}

class _ScopePainter extends CustomPainter {
  const _ScopePainter(this.m, this.waveform);

  final TantrumMetrics m;
  final List<double> waveform;

  @override
  void paint(Canvas canvas, Size size) {
    final s = m.scale;
    final full = Offset.zero & size;
    // 屏坐在沉槽里：整块画布先做内阴影，屏幕本体再往里缩 well
    paintRecess(
        canvas, RRect.fromRectAndRadius(full, Radius.circular(m.radius * 0.5)), s, depth: 1.2);
    final rect = full.deflate(m.well);
    final clip = RRect.fromRectAndRadius(rect, Radius.circular(7 * s));
    canvas.save();
    canvas.clipRRect(clip);
    canvas.drawRect(
        rect,
        Paint()
          ..shader = faceShader(rect, const [
                TantrumTokens.screenLight,
                TantrumTokens.screenTop,
                TantrumTokens.screenShade,
              ], light: const Alignment(-0.35, -1)));
    // 玻璃四角的暗角，屏幕才有"陷在机箱里"的深度
    canvas.drawRect(
      rect,
      Paint()
        ..shader = RadialGradient(
          radius: 0.78,
          colors: [const Color(0x00000000), const Color(0x59000000)],
          stops: const [0.6, 1.0],
        ).createShader(rect),
    );

    final hairline = Paint()
      ..color = TantrumTokens.screenGrid
      ..strokeWidth = 1;
    // 上半区：方格
    for (var i = 1; i < 8; i++) {
      final x = rect.left + rect.width * i / 8;
      canvas.drawLine(Offset(x, rect.top), Offset(x, rect.top + rect.height * 0.58), hairline);
    }
    for (var i = 1; i < 4; i++) {
      final y = rect.top + rect.height * 0.58 * i / 4;
      canvas.drawLine(Offset(rect.left, y), Offset(rect.right, y), hairline);
    }
    // 下半区：透视线地板（图1 的屏底网格）
    final floor = Paint()
      ..color = TantrumTokens.screenGridFloor
      ..strokeWidth = 1;
    final horizon = rect.top + rect.height * 0.58;
    for (var i = 0; i <= 6; i++) {
      final t = i / 6;
      final topX = rect.left + rect.width * (0.18 + 0.64 * t);
      final bottomX = rect.left + rect.width * (-0.35 + 1.7 * t);
      canvas.drawLine(Offset(topX, horizon), Offset(bottomX, rect.bottom), floor);
    }
    for (var i = 1; i <= 4; i++) {
      final y = horizon + (rect.bottom - horizon) * math.pow(i / 4, 1.9).toDouble();
      canvas.drawLine(Offset(rect.left, y), Offset(rect.right, y), floor);
    }

    // 波形
    if (waveform.isNotEmpty) {
      final base = rect.top + (horizon - rect.top) * 0.60;
      final amp = (horizon - rect.top) * 0.24;
      final path = Path()..moveTo(rect.left, base);
      for (var i = 0; i < waveform.length; i++) {
        path.lineTo(rect.left + rect.width * i / (waveform.length - 1), base - waveform[i] * amp);
      }
      // 中轴：没有它，方波像贴在玻璃上的一条胶带
      canvas.drawLine(
        Offset(rect.left, base),
        Offset(rect.right, base),
        Paint()
          ..color = TantrumTokens.screenGrid
          ..strokeWidth = 1,
      );
      // 荧光粉的余辉：宽而淡的一层，再压一条细亮线
      canvas.drawPath(
        path,
        Paint()
          ..color = TantrumTokens.screenInk.withValues(alpha: 0.22)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 5 * s
          ..strokeJoin = StrokeJoin.round
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2.2 * s),
      );
      canvas.drawPath(
        path.shift(Offset(0, 1.4 * s)),
        Paint()
          ..color = const Color(0x40000000)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 2.4 * s,
      );
      canvas.drawPath(
        path,
        Paint()
          ..color = TantrumTokens.screenInk
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.5 * s
          ..strokeJoin = StrokeJoin.round,
      );
    }
    canvas.restore();
    // 玻璃：左上到右下的一道反光带，屏幕是罩在面板下的而不是印上去的
    canvas.save();
    canvas.clipRRect(clip);
    final sheen = Rect.fromLTRB(rect.left, rect.top, rect.right, rect.top + rect.height * 0.42);
    canvas.drawRect(
      sheen,
      Paint()
        ..shader = LinearGradient(
          begin: const Alignment(-0.9, -1),
          end: const Alignment(0.4, 1),
          colors: [const Color(0x1FFFFFFF), const Color(0x00FFFFFF)],
        ).createShader(sheen),
    );
    canvas.restore();
    // 玻璃外沿的一圈暗边
    canvas.drawRRect(
      clip,
      Paint()
        ..color = const Color(0x66000000)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_ScopePainter old) => old.waveform != waveform || old.m.scale != m.scale;
}

/// 右格栅：同心点阵 + 左上红扇区 + 中心圆（图1 的喇叭罩）
class TantrumGrille extends StatelessWidget {
  const TantrumGrille({super.key, required this.m, required this.centerLabel});

  final TantrumMetrics m;
  final String centerLabel;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: EdgeInsets.only(left: m.well, top: m.well),
      child: CustomPaint(
        painter: _GrillePainter(m),
        // 中心那颗轴心是凸的，字压在它上面会糊成一团：丝印挪到板子下沿
        child: Align(
          alignment: Alignment.bottomCenter,
          child: Padding(
            padding: EdgeInsets.only(bottom: 5 * m.scale),
            child: Text(centerLabel,
                style: TantrumTokens.caption(m.scale, color: TantrumTokens.inkFaint)),
          ),
        ),
      ),
    );
  }
}

class _GrillePainter extends CustomPainter {
  const _GrillePainter(this.m);

  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final s = m.scale;
    final rect = Offset.zero & size;
    final clip = RRect.fromRectAndRadius(rect, Radius.circular(7 * s));
    // 格栅是凸出面板的一块塑料板
    paintRaise(canvas, clip, s, depth: 1.15);
    canvas.save();
    canvas.clipRRect(clip);
    canvas.drawRect(
        rect,
        Paint()
          ..shader = faceShader(rect, const [
                TantrumTokens.grilleLight,
                TantrumTokens.grilleShade,
              ], light: const Alignment(-0.5, -1)));
    final center = Offset(size.width * 0.5, size.height * 0.45);
    // 点阵只能按短边铺：按宽度算的话，上下两圈的孔会被板边拦腰切断。
    final maxR = size.height * 0.38;
    var ring = 1;
    for (var r = maxR * 0.2; r < maxR; r += maxR * 0.08) {
      final count = (2 * math.pi * r / (maxR * 0.07)).ceil();
      for (var i = 0; i < count; i++) {
        final angle = 2 * math.pi * i / count - math.pi / 2;
        final p = center + Offset(math.cos(angle), math.sin(angle)) * r;
        // 图1：左上扇区的点是红的（画布 y 轴向下，π=左、1.5π=上）
        final inWedge = angle > math.pi * 1.02 && angle < math.pi * 1.42;
        final holeR = size.width * 0.0048 + ring * 0.03;
        // 孔是打进去的：下缘先回白（塑料被掀起的边），孔腔本身压到比板面暗两档，
        // 只点一个浅灰圆看起来像印上去的芝麻。
        canvas.drawCircle(
          p + Offset(0, 0.9 * s),
          holeR * 1.12,
          Paint()..color = TantrumTokens.rimLight.withValues(alpha: 0.9),
        );
        canvas.drawCircle(
          p,
          holeR,
          Paint()
            ..shader = LinearGradient(
              begin: Alignment.topCenter,
              end: Alignment.bottomCenter,
              colors: inWedge
                  ? [const Color(0xB3C0311C), TantrumTokens.accent]
                  : [const Color(0xFF7C8695), const Color(0xFFA5AEBA)],
            ).createShader(Rect.fromCircle(center: p, radius: holeR)),
        );
      }
      ring++;
    }
    // 中心：凹进去的轴心 + 一颗凸出的小圆盖 + 四个刻度
    final core = size.width * 0.055;
    canvas.drawCircle(center, core * 1.3,
        Paint()..color = TantrumTokens.grilleShade);
    paintRecessCircle(canvas, center, core * 1.3, s, depth: 1.0);
    paintRaiseCircle(canvas, center, core * 0.86, s, depth: 0.7);
    canvas.drawCircle(
      center,
      core * 0.86,
      Paint()
        ..shader = faceShader(
            Rect.fromCircle(center: center, radius: core * 0.86),
            const [Colors.white, TantrumTokens.meterFloor],
            light: const Alignment(-0.4, -1)),
    );
    paintBevel(canvas,
        RRect.fromRectAndRadius(
            Rect.fromCircle(center: center, radius: core * 0.86), Radius.circular(core * 0.86)),
        s,
        width: 0.8);
    final tick = Paint()
      ..color = TantrumTokens.ink
      ..strokeWidth = 1.4
      ..strokeCap = StrokeCap.round;
    for (var i = 0; i < 4; i++) {
      final angle = i * math.pi / 2 + math.pi / 4;
      final dir = Offset(math.cos(angle), math.sin(angle));
      canvas.drawLine(center + dir * (core * 1.5), center + dir * (core * 2.0), tick);
    }
    canvas.restore();
    paintBevel(canvas, clip, s, width: 1.3);
  }

  @override
  bool shouldRepaint(covariant _GrillePainter old) => old.m.scale != m.scale;
}

/// EQ 图窗（图2 中间卡片：网格 + 虚线 0 dB + 频谱灰区 + 橙色曲线）
class TantrumEqGraph extends StatelessWidget {
  const TantrumEqGraph({super.key, required this.m, required this.curve, required this.spectrum});

  final TantrumMetrics m;
  final List<double> curve;
  final List<double> spectrum;

  @override
  Widget build(BuildContext context) {
    return CustomPaint(painter: _EqPainter(m, curve, spectrum));
  }
}

class _EqPainter extends CustomPainter {
  const _EqPainter(this.m, this.curve, this.spectrum);

  final TantrumMetrics m;
  final List<double> curve;
  final List<double> spectrum;

  @override
  void paint(Canvas canvas, Size size) {
    final s = m.scale;
    final rect = Offset.zero & size;
    final clip = RRect.fromRectAndRadius(rect, Radius.circular(6 * s));
    // 图窗是凹进卡片的玻璃窗：窗底要和面板拉开一档明度，否则"凹"字无从谈起
    paintRecess(canvas, clip, s, depth: 1.3);
    canvas.save();
    canvas.clipRRect(clip);
    canvas.drawRect(
        rect,
        Paint()
          ..shader = faceShader(rect, const [
                TantrumTokens.meterFloor,
                TantrumTokens.wellBottom,
              ], light: const Alignment(-0.3, -1)));

    final grid = Paint()
      ..color = TantrumTokens.tickIdle
      ..strokeWidth = 1;
    for (var i = 1; i < 5; i++) {
      final x = size.width * i / 5;
      canvas.drawLine(Offset(x, 0), Offset(x, size.height), grid);
    }
    for (var i = 1; i < 4; i++) {
      final y = size.height * i / 4;
      canvas.drawLine(Offset(0, y), Offset(size.width, y), grid);
    }
    // 0 dB 虚线
    final zero = Paint()
      ..color = TantrumTokens.wellInk
      ..strokeWidth = 1;
    final mid = size.height * 0.42;
    for (var x = 0.0; x < size.width; x += 5 * s) {
      canvas.drawLine(Offset(x, mid), Offset(math.min(x + 2.5 * s, size.width), mid), zero);
    }

    // 频谱灰区
    if (spectrum.isNotEmpty) {
      final path = Path()..moveTo(0, size.height);
      for (var i = 0; i < spectrum.length; i++) {
        path.lineTo(size.width * i / (spectrum.length - 1),
            size.height - spectrum[i] * size.height * 0.85);
      }
      path.lineTo(size.width, size.height);
      canvas.drawPath(
        path,
        Paint()
          ..shader = LinearGradient(
            begin: Alignment.topCenter,
            end: Alignment.bottomCenter,
            colors: [
              TantrumTokens.wellInk.withValues(alpha: 0.30),
              TantrumTokens.tickIdle.withValues(alpha: 0.55),
            ],
          ).createShader(rect),
      );
    }
    // 橙色曲线（±12 dB 映射到面板高度）：先一道暗投影再压亮线，曲线才浮在窗面上
    if (curve.length > 1) {
      final path = Path();
      for (var i = 0; i < curve.length; i++) {
        final x = size.width * i / (curve.length - 1);
        final y = mid - curve[i] * size.height * 0.34;
        i == 0 ? path.moveTo(x, y) : path.lineTo(x, y);
      }
      canvas.drawPath(
        path.shift(Offset(0, 1.6 * s)),
        Paint()
          ..color = const Color(0x332C3138)
          ..style = PaintingStyle.stroke
          ..strokeWidth = 3.2 * s
          ..strokeJoin = StrokeJoin.round
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, 1.2 * s),
      );
      canvas.drawPath(
        path,
        Paint()
          ..color = TantrumTokens.curve
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.6 * s
          ..strokeJoin = StrokeJoin.round,
      );
    }
    canvas.restore();
    canvas.drawRRect(
      clip,
      Paint()
        ..color = TantrumTokens.knobEdge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1,
    );
  }

  @override
  bool shouldRepaint(_EqPainter old) =>
      old.curve != curve || old.spectrum != spectrum || old.m.scale != m.scale;
}

/// EQ 图窗 + 四推子（图2 中间卡片的整体）
class TantrumEqBank extends StatelessWidget {
  const TantrumEqBank({
    super.key,
    required this.m,
    required this.bands,
    required this.curve,
    required this.spectrum,
    this.onBand,
  });

  final TantrumMetrics m;
  final List<double> bands;
  final List<double> curve;
  final List<double> spectrum;
  final void Function(int index, double value)? onBand;

  static const _labels = ['低频', '中频', '高频', '空气'];

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          height: m.bodyHeight * 0.42,
          width: double.infinity,
          child: TantrumEqGraph(m: m, curve: curve, spectrum: spectrum),
        ),
        SizedBox(height: 8 * m.scale),
        Row(
          mainAxisAlignment: MainAxisAlignment.spaceEvenly,
          children: [
            for (var i = 0; i < bands.length; i++)
              TantrumFader(
                value: bands[i],
                m: m,
                label: _labels[i],
                onChanged: onBand == null ? null : (v) => onBand!(i, v),
              ),
          ],
        ),
      ],
    );
  }
}

/// 深色标题条：品牌字 + 预设切换 + 齿轮 + 电源（图1 顶栏）
class TantrumHeader extends StatelessWidget {
  const TantrumHeader({
    super.key,
    required this.m,
    required this.brand,
    required this.preset,
    this.onPrev,
    this.onNext,
    this.onGear,
    this.onPower,
    this.powerOn = true,
  });

  final TantrumMetrics m;
  final String brand;
  final String preset;
  final VoidCallback? onPrev;
  final VoidCallback? onNext;
  final VoidCallback? onGear;
  final VoidCallback? onPower;
  final bool powerOn;

  @override
  Widget build(BuildContext context) {
    return Container(
      height: m.headerHeight,
      padding: EdgeInsets.symmetric(horizontal: 10 * m.scale),
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(7 * m.scale),
        gradient: const LinearGradient(
          begin: Alignment(-0.2, -1),
          end: Alignment(0.2, 1),
          colors: [TantrumTokens.headerTop, TantrumTokens.headerBottom],
        ),
        boxShadow: [
          // 标题条是浮在面板上的深色条：右下投影 + 左上回白
          BoxShadow(
            color: TantrumTokens.castDark,
            blurRadius: 6 * m.scale,
            offset: Offset(1.4 * m.scale, 2.6 * m.scale),
          ),
          BoxShadow(
            color: TantrumTokens.rimLight,
            blurRadius: 3 * m.scale,
            offset: Offset(-1 * m.scale, -1.2 * m.scale),
          ),
        ],
      ),
      child: Stack(
        children: [
          // 顶边一条回白，深色条才不像一块贴上去的矩形
          Positioned(
            left: 0,
            right: 0,
            top: 0,
            height: 1,
            child: DecoratedBox(
                decoration: const BoxDecoration(
                    color: Color(0x2EFFFFFF),
                    borderRadius: BorderRadius.vertical(top: Radius.circular(7)))),
          ),
          Padding(
            padding: EdgeInsets.symmetric(horizontal: 0),
            child: Row(
              children: [
                Text(brand, style: TantrumTokens.brand(m.scale)),
                SizedBox(width: 6 * m.scale),
                const _BrandGlyph(),
                const Spacer(),
                Icon(Icons.chevron_left_rounded,
                        size: 14 * m.scale, color: TantrumTokens.inkOnDark)
                    .gesture(onPrev),
                SizedBox(width: 4 * m.scale),
                Text(preset, style: TantrumTokens.preset(m.scale)),
                SizedBox(width: 4 * m.scale),
                Icon(Icons.chevron_right_rounded,
                        size: 14 * m.scale, color: TantrumTokens.inkOnDark)
                    .gesture(onNext),
                SizedBox(width: 10 * m.scale),
                TantrumRoundButton(
                  m: m,
                  icon: Icons.settings_rounded,
                  onDark: true,
                  onTap: onGear,
                ),
                SizedBox(width: 6 * m.scale),
                TantrumRoundButton(
                  m: m,
                  icon: Icons.power_settings_new_rounded,
                  onDark: true,
                  active: powerOn,
                  onTap: onPower,
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

extension on Widget {
  Widget gesture(VoidCallback? tap) =>
      GestureDetector(onTap: tap, behavior: HitTestBehavior.opaque, child: this);
}

/// 品牌字右侧的小波形丝印（图1 的 TANTRUM···）
class _BrandGlyph extends StatelessWidget {
  const _BrandGlyph();

  @override
  Widget build(BuildContext context) {
    return CustomPaint(
      size: const Size(26, 10),
      painter: _GlyphPainter(),
    );
  }
}

class _GlyphPainter extends CustomPainter {
  @override
  void paint(Canvas canvas, Size size) {
    final path = Path()
      ..moveTo(0, size.height * 0.5)
      ..lineTo(size.width * 0.18, size.height * 0.5)
      ..lineTo(size.width * 0.28, size.height * 0.12)
      ..lineTo(size.width * 0.42, size.height * 0.88)
      ..lineTo(size.width * 0.56, size.height * 0.3)
      ..lineTo(size.width * 0.7, size.height * 0.7)
      ..lineTo(size.width, size.height * 0.5);
    canvas.drawPath(
      path,
      Paint()
        ..color = TantrumTokens.inkOnDark
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.2
        ..strokeJoin = StrokeJoin.round,
    );
  }

  @override
  bool shouldRepaint(covariant CustomPainter oldDelegate) => false;
}

/// 面板外壳：桌面渐变 + 白塑面 + 顶边高光（图1 的机箱）
class TantrumShell extends StatelessWidget {
  const TantrumShell({super.key, required this.width, required this.child});

  final double width;
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final m = TantrumMetrics(width);
    return Container(
      decoration: const BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.deskTop, TantrumTokens.deskBottom],
        ),
      ),
      alignment: Alignment.center,
      child: SizedBox(
        width: width,
        height: m.height,
        child: DecoratedBox(
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(m.radius),
            gradient: const LinearGradient(
              begin: Alignment(-0.35, -1),
              end: Alignment(0.35, 1),
              colors: [TantrumTokens.panelTop, TantrumTokens.panelBottom],
            ),
            boxShadow: [
              // 机箱浮在桌面上：右下大投影 + 左上回白
              BoxShadow(
                color: TantrumTokens.castDark,
                blurRadius: 30 * m.scale,
                offset: Offset(6 * m.scale, 12 * m.scale),
              ),
              BoxShadow(
                color: TantrumTokens.rimLight,
                blurRadius: 20 * m.scale,
                offset: Offset(-5 * m.scale, -6 * m.scale),
              ),
            ],
          ),
          child: ClipRRect(
            borderRadius: BorderRadius.circular(m.radius),
            child: Stack(
              children: [
                Positioned(
                  left: 0,
                  right: 0,
                  top: 0,
                  height: 1.4,
                  child: DecoratedBox(
                      decoration: const BoxDecoration(color: TantrumTokens.panelBevel)),
                ),
                // 下缘的一抹暗边让机箱有厚度，而不是白色圆角矩形
                Positioned(
                  left: 0,
                  right: 0,
                  bottom: 0,
                  height: 2.6,
                  child: DecoratedBox(
                      decoration: BoxDecoration(
                          gradient: LinearGradient(
                            begin: Alignment.topCenter,
                            end: Alignment.bottomCenter,
                            colors: [
                              const Color(0x008C98A8),
                              TantrumTokens.rimShade,
                            ],
                          ))),
                ),
                child,
              ],
            ),
          ),
        ),
      ),
    );
  }
}
