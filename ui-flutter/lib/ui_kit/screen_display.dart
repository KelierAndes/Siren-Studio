import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'model.dart';
import 'theme.dart';

/// Mix screen: one EQ curve per receiving device.
///
/// The curve height is `band level × channel gain` (linear), so turning a
/// device's fader up lifts its whole curve. Band levels come from the Rust
/// receiver's per-stream 1024-point FFT.
class ScreenDisplay extends StatelessWidget {
  const ScreenDisplay({
    super.key,
    required this.model,
    required this.palette,
    required this.size,
    this.radius = 16,
  });

  final ShaperModel model;
  final MetalPalette palette;
  final Size size;
  final double radius;

  static const List<double> rulerHz = [
    20,
    50,
    100,
    200,
    500,
    1000,
    2000,
    5000,
    10000,
    16000,
  ];

  static String rulerLabel(double hz) =>
      hz >= 1000 ? '${(hz / 1000).round()}k' : hz.round().toString();

  @override
  Widget build(BuildContext context) {
    return NeuWell(
      size: size,
      radius: radius,
      palette: palette,
      deep: true,
      sheen: true,
      child: ListenableBuilder(
        listenable: model,
        builder: (context, _) => CustomPaint(
          size: size,
          painter: _EqPainter(
            palette: palette,
            curves: model.eqCurves,
          ),
        ),
      ),
    );
  }
}

class _EqPainter extends CustomPainter {
  _EqPainter({required this.palette, required this.curves});

  final MetalPalette palette;
  final List<EqCurve> curves;

  @override
  void paint(Canvas canvas, Size size) {
    final plot = Rect.fromLTRB(12, 32, size.width - 12, size.height - 14);
    final ink = palette.inkFaint;
    final grid = Paint()
      ..color = palette.inkFaint.withValues(alpha: .16)
      ..strokeWidth = 1;

    for (var i = 1; i < 4; i++) {
      final y = plot.top + plot.height * i / 4;
      canvas.drawLine(Offset(plot.left, y), Offset(plot.right, y), grid);
    }
    for (final hz in ScreenDisplay.rulerHz) {
      final x = plot.left + hzToX(hz) * plot.width;
      canvas.drawLine(
        Offset(x, plot.top),
        Offset(x, plot.top + 5),
        grid..strokeWidth = 1.2,
      );
      _label(
        canvas,
        ScreenDisplay.rulerLabel(hz),
        Offset(x, plot.top - 13),
        NeuType.chrome(8.5, ink, tracking: .4),
      );
    }

    // Watermark, matching the reference screen.
    _label(
      canvas,
      'S I R E N :',
      Offset(plot.center.dx - 48, plot.top + 34),
      NeuType.chrome(17, ink.withValues(alpha: .28), tracking: 2.2, w: FontWeight.w600),
    );

    for (final curve in curves) {
      final bands = curve.bands;
      if (bands.length < 2) continue;
      final gain = math.pow(10, curve.gainDb / 20).toDouble().clamp(0.0, 1.6);
      final path = Path();
      final points = <Offset>[];
      for (var i = 0; i < bands.length; i++) {
        final t = (i + .5) / bands.length;
        final hz = kScreenMinHz * math.pow(kScreenMaxHz / kScreenMinHz, t);
        final x = plot.left + hzToX(hz) * plot.width;
        final level = (bands[i] * gain).clamp(0.0, 1.0);
        final y = plot.bottom - 1 - level * plot.height * .86;
        points.add(Offset(x, y));
      }
      path.moveTo(plot.left, points.first.dy);
      path.lineTo(points.first.dx, points.first.dy);
      for (var i = 1; i < points.length; i++) {
        final prev = points[i - 1];
        final current = points[i];
        final mid = Offset((prev.dx + current.dx) / 2, (prev.dy + current.dy) / 2);
        path.quadraticBezierTo(prev.dx, prev.dy, mid.dx, mid.dy);
        if (i == points.length - 1) path.lineTo(current.dx, current.dy);
      }
      final fill = Path.from(path)
        ..lineTo(plot.right, plot.bottom)
        ..lineTo(plot.left, plot.bottom)
        ..close();
      canvas.drawPath(
        fill,
        Paint()
          ..style = PaintingStyle.fill
          ..color = curve.color.withValues(alpha: curve.active ? .16 : .06),
      );
      canvas.drawPath(
        path,
        Paint()
          ..style = PaintingStyle.stroke
          ..strokeWidth = 1.8
          ..strokeJoin = StrokeJoin.round
          ..color = curve.color.withValues(alpha: curve.active ? .95 : .4),
      );
    }

    // Legend (up to four, top-left under the ruler).
    var legendX = plot.left + 2;
    for (final curve in curves.take(4)) {
      final rect = Rect.fromLTWH(legendX, plot.top + 4, 7, 7);
      canvas.drawRRect(
        RRect.fromRectAndRadius(rect, const Radius.circular(2)),
        Paint()..color = curve.color,
      );
      final painter = _text(
        curve.name,
        NeuType.chrome(10, palette.ink.withValues(alpha: .8), tracking: .2, w: FontWeight.w600),
      );
      if (legendX + 11 + painter.width > plot.right - 4) break;
      painter.paint(canvas, Offset(legendX + 11, plot.top + 1));
      legendX += 11 + painter.width + 14;
    }
  }

  TextPainter _text(String value, TextStyle style) {
    final painter = TextPainter(
      text: TextSpan(text: value, style: style),
      textDirection: TextDirection.ltr,
      maxLines: 1,
      ellipsis: '…',
    )..layout(maxWidth: 160);
    return painter;
  }

  void _label(Canvas canvas, String value, Offset at, TextStyle style) {
    _text(value, style).paint(canvas, at);
  }

  @override
  bool shouldRepaint(_EqPainter old) => true;
}
