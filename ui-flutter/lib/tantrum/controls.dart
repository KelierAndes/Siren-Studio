import 'dart:math' as math;

import 'package:flutter/material.dart';

import 'tokens.dart';

/// 参考图 2 的控件 token 实现：点阵刻度环旋钮、帽式推子、分段胶囊、
/// ADV 开关、圆形电源/齿轮钮、LED。只画不改数据，值由回调交回上层。

/// 旋钮：0.0–1.0 归一化值，-135°…+135° 行程。
class TantrumKnob extends StatelessWidget {
  const TantrumKnob({
    super.key,
    required this.value,
    required this.m,
    this.label,
    this.valueText,
    this.onChanged,
  });

  final double value;
  final TantrumMetrics m;
  final String? label;
  final String? valueText;
  final ValueChanged<double>? onChanged;

  static const double _sweep = 270;

  @override
  Widget build(BuildContext context) {
    final d = m.knobDiameter;
    return GestureDetector(
      onVerticalDragUpdate: onChanged == null
          ? null
          : (delta) => onChanged!((value - delta.delta dy * 0).clamp(0, 1)),
      child: SizedBox(
        width: m.knobRingRadius * 2 + 4,
        height: d + 4,
        child: CustomPaint(painter: _KnobPainter(value, m)),
      ),
    );
  }
}

class _KnobPainter extends CustomPainter {
  const _KnobPainter(this.value, this.m);

  final double value;
  final TantrumMetrics m;
  static const double _start = math.pi * 0.75;

  @override
  void paint(Canvas canvas, Size size) {
    final center = Offset(size.width / 2, size.height / 2);
    final r = m.knobDiameter / 2;

    // 点阵刻度环（图2：环半径 = 旋钮半径 × 1.14，已走过的段用深灰）
    final ringR = m.knobRingRadius;
    final ticks = m.tickCount.toInt();
    final active = (value * (ticks - 1)).round();
    for (var i = 0; i < ticks; i++) {
      final t = i / (ticks - 1);
      final angle = _start + t * math.pi * 1.5;
      final painted = i <= active;
      canvas.drawCircle(
        center + Offset(math.cos(angle), math.sin(angle)) * ringR,
        m.tickRadius * (painted ? 1.0 : 0.8),
        Paint()
          ..color = painted ? TantrumTokens.tickActive : TantrumTokens.tickIdle
          ..style = PaintingStyle.fill,
      );
    }

    // 塑面投影 + 本体
    canvas.drawCircle(
      center + Offset(0, m.scale * 2.2),
      r,
      Paint()
        ..color = TantrumTokens.knobShadow
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 3.4 * m.scale),
    );
    final face = Paint()
      ..shader = const LinearGradient(
        begin: Alignment.topCenter,
        end: Alignment.bottomCenter,
        colors: [TantrumTokens.knobTop, TantrumTokens.knobBottom],
      ).createShader(Rect.fromCircle(center: center, radius: r));
    canvas.drawCircle(center, r, face);
    canvas.drawCircle(
      center,
      r,
      Paint()
        ..color = TantrumTokens.knobEdge
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1 * m.scale,
    );
    // 顶边高光
    canvas.drawArc(
      Rect.fromCircle(center: center, radius: r - 0.8 * m.scale),
      math.pi * 1.12,
      math.pi * 0.76,
      false,
      Paint()
        ..color = const Color(0x99FFFFFF)
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1.4 * m.scale
        ..strokeCap = StrokeCap.round,
    );

    // 指示槽：深色胶囊，落在 0.86 半径（图2 实测 0.94 处是槽心）
    final angle = _start + value * math.pi * 1.5;
    final dir = Offset(math.cos(angle), math.sin(angle));
    final mid = center + dir * m.knobNotchRadius;
    canvas.save();
    canvas.translate(mid.dx, mid.dy);
    canvas.rotate(angle + math.pi / 2);
    RRect.draw(
      canvas,
      RRect.fromRectAndRadius(
        Rect.fromCenter(
          center: Offset.zero,
          width: m.knobNotchWidth,
          height: m.knobNotchLength,
        ),
        Radius.circular(m.knobNotchWidth / 2),
      ),
      Paint()..color = TantrumTokens.notch,
    );
    canvas.restore();
  }

  @override
  bool shouldRepaint(_KnobPainter old) => old.value != value || old.m != m;
}

/// 帽式推子（图2：槽宽 5、帽 26×13、行程 120）
class TantrumFader extends StatelessWidget {
  const TantrumFader({
    super.key,
    required this.value,
    required this.m,
    required this.label,
    this.onChanged,
  });

  final double value;
  final TantrumMetrics m;
  final String label;
  final ValueChanged<double>? onChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      children: [
        SizedBox(
          width: m.faderCapWidth + 6,
          height: m.faderTravel + 8,
          child: GestureDetector(
            onVerticalDragUpdate: onChanged == null
                ? null
                : (d) => onChanged!((value - d.deltaDy / m.faderTravel).clamp(0, 1)),
            child: CustomPaint(painter: _FaderPainter(value, m)),
          ),
        ),
        SizedBox(height: 5 * m.scale),
        Text(label, style: TantrumTokens.caption(m.scale, color: TantrumTokens.ink)),
      ],
    );
  }
}

class _FaderPainter extends CustomPainter {
  const _FaderPainter(this.value, this.m);

  final double value;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final cx = size.width / 2;
    final top = 4.0;
    final bottom = size.height - 4.0;
    final travel = bottom - top;
    // 槽
    canvas.drawRRect(
      RRect.fromRectAndRadius(
        Rect.fromLTWH(cx - m.faderSlotWidth / 2, top, m.faderSlotWidth, travel),
        Radius.circular(m.faderSlotWidth / 2),
      ),
      Paint()..color = TantrumTokens.slot,
    );
    final capY = bottom - (bottom - top) * value - m.faderCapHeight / 2;
    final cap = Rect.fromCenter(
      center: Offset(cx, capY),
      width: m.faderCapWidth,
      height: m.faderCapHeight,
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap.shift(Offset(0, 2 * m.scale)), Radius.circular(4 * m.scale)),
      Paint()
        ..color = TantrumTokens.knobShadow
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2.2 * m.scale),
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(cap, Radius.circular(4 * m.scale)),
      Paint()
        ..shader = const LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [TantrumTokens.capTop, TantrumTokens.capBottom],
        ).createShader(cap),
    );
    // 帽上的深色握线
    canvas.drawLine(
      Offset(cap.left + 5 * m.scale, cap.center.dy),
      Offset(cap.right - 5 * m.scale, cap.center.dy),
      Paint()
        ..color = TantrumTokens.notch
        ..strokeWidth = 2 * m.scale
        ..strokeCap = StrokeCap.round,
    );
  }

  @override
  bool shouldRepaint(_FaderPainter old) => old.value != value || old.m != m;
}

/// 分段胶囊（图2：Fast / Slow / Auto）
class TantrumSegPill extends StatelessWidget {
  const TantrumSegPill({
    super.key,
    required this.items,
    required this.index,
    required this.m,
    this.onSelect,
  });

  final List<String> items;
  final int index;
  final TantrumMetrics m;
  final ValueChanged<int>? onSelect;

  @override
  Widget build(BuildContext context) {
    final w = m.segSegmentWidth * items.length + 6 * m.scale;
    return GestureDetector(
      onTapUp: onSelect == null
          ? null
          : (d) => onSelect!(((d.localPosition.dx - 3 * m.scale) / m.segSegmentWidth)
              .floor()
              .clamp(0, items.length - 1)),
      child: SizedBox(
        width: w,
        height: m.segHeight,
        child: CustomPaint(painter: _SegPainter(items, index, m)),
      ),
    );
  }
}

class _SegPainter extends CustomPainter {
  const _SegPainter(this.items, this.index, this.m);

  final List<String> items;
  final int index;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final track = Offset(3 * m.scale, 0) & Size(size.width - 6 * m.scale, size.height);
    canvas.drawRRect(
      RRect.fromRectAndRadius(track, Radius.circular(size.height / 2)),
      Paint()..color = TantrumTokens.segTrack,
    );
    final w = track.width / items.length;
    final active = Rect.fromLTWH(track.left + index * w + 1.5 * m.scale, 1.5 * m.scale,
        w - 3 * m.scale, size.height - 3 * m.scale);
    canvas.drawRRect(
      RRect.fromRectAndRadius(active, Radius.circular(active.height / 2)),
      Paint()
        ..color = Colors.white
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 1.6 * m.scale),
    );
    canvas.drawRRect(
      RRect.fromRectAndRadius(active, Radius.circular(active.height / 2)),
      Paint()..color = TantrumTokens.segOn,
    );
    for (var i = 0; i < items.length; i++) {
      final tp = TextPainter(
        text: TextSpan(
          text: items[i],
          style: TantrumTokens.caption(
            m.scale * 1.12,
            color: i == index ? TantrumTokens.inkStrong : TantrumTokens.inkFaint,
          ),
        ),
        textDirection: TextDirection.ltr,
      )..layout();
      tp.paint(
        canvas,
        Offset(track.left + i * w + (w - tp.width) / 2, (size.height - tp.height) / 2),
      );
    }
  }

  @override
  bool shouldRepaint(_SegPainter old) => old.index != index || old.m != m;
}

/// ADV 开关（图2：56×28 轨道 + 22 滑块；开 = 深色轨道）
class TantrumSwitch extends StatelessWidget {
  const TantrumSwitch({super.key, required this.on, required this.m, this.onChanged});

  final bool on;
  final TantrumMetrics m;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onChanged == null ? null : () => onChanged!(!on),
      child: SizedBox(
        width: m.switchWidth,
        height: m.switchHeight,
        child: CustomPaint(painter: _SwitchPainter(on, m)),
      ),
    );
  }
}

class _SwitchPainter extends CustomPainter {
  const _SwitchPainter(this.on, this.m);

  final bool on;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final track = RRect.fromRectAndRadius(
      Offset.zero & size,
      Radius.circular(size.height / 2),
    );
    canvas.drawRRect(track, Paint()..color = on ? TantrumTokens.switchOn : TantrumTokens.switchOff);
    final r = size.height / 2 - 2 * m.scale;
    final center = Offset(on ? size.width - r - 2 * m.scale : r + 2 * m.scale, size.height / 2);
    canvas.drawCircle(
      center + Offset(0, 1.4 * m.scale),
      r,
      Paint()
        ..color = TantrumTokens.panelShadow
        ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2 * m.scale),
    );
    canvas.drawCircle(center, r, Paint()..color = Colors.white);
  }

  @override
  bool shouldRepaint(_SwitchPainter old) => old.on != on || old.m != m;
}

/// 圆形凹陷按钮（电源、齿轮）
class TantrumRoundButton extends StatelessWidget {
  const TantrumRoundButton({
    super.key,
    required this.m,
    required this.icon,
    this.onDark = false,
    this.onTap,
  });

  final TantrumMetrics m;
  final IconData icon;
  final bool onDark;
  final VoidCallback? onTap;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: onTap,
      child: SizedBox(
        width: m.roundButton,
        height: m.roundButton,
        child: CustomPaint(painter: _RoundBtnPainter(icon, m, onDark)),
      ),
    );
  }
}

class _RoundBtnPainter extends CustomPainter {
  const _RoundBtnPainter(this.icon, this.m, this.onDark);

  final IconData icon;
  final TantrumMetrics m;
  final bool onDark;

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    final r = size.width / 2;
    if (!onDark) {
      canvas.drawCircle(
        center + Offset(0, 1.2 * m.scale),
        r,
        Paint()
          ..color = TantrumTokens.knobShadow
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2 * m.scale),
      );
      canvas.drawCircle(center, r, Paint()..color = Colors.white);
    }
    final tp = TextPainter(
      text: IconTheme.merge(
        data: IconThemeData(
          size: r * 1.05,
          color: onDark ? TantrumTokens.inkOnDark : TantrumTokens.ink,
        ),
        child: Text(icon),
      ) as TextSpan,
      textDirection: TextDirection.ltr,
    )..layout();
    tp.paint(canvas, center - Offset(tp.width / 2, tp.height / 2));
  }

  @override
  bool shouldRepaint(_RoundBtnPainter old) =>
      old.icon != icon || old.m != m || old.onDark != onDark;
}

/// LED（图1 里旋钮之间那颗红点）
class TantrumLed extends StatelessWidget {
  const TantrumLed({super.key, required this.on, required this.m});

  final bool on;
  final TantrumMetrics m;

  @override
  Widget build(BuildContext context) {
    final d = m.ledDiameter;
    return SizedBox(
      width: d,
      height: d,
      child: CustomPaint(
        painter: _LedPainter(on, m),
      ),
    );
  }
}

class _LedPainter extends CustomPainter {
  const _LedPainter(this.on, this.m);

  final bool on;
  final TantrumMetrics m;

  @override
  void paint(Canvas canvas, Size size) {
    final center = size.center(Offset.zero);
    canvas.drawCircle(
      center,
      size.width / 2,
      Paint()..color = on ? TantrumTokens.ledOff : TantrumTokens.ledOff,
    );
    if (on) {
      canvas.drawCircle(
        center,
        size.width / 2 - 1 * m.scale,
        Paint()
          ..color = TantrumTokens.ledOn
          ..maskFilter = MaskFilter.blur(BlurStyle.normal, 2.4 * m.scale),
      );
    }
    canvas.drawCircle(
      center,
      size.width / 2 - 1.2 * m.scale,
      Paint()..color = on ? TantrumTokens.ledOn : TantrumTokens.ledOff,
    );
  }

  @override
  bool shouldRepaint(_LedPainter old) => old.on != on || old.m != m;
}

/// 上下箭头一对（图1 中间那一组）
class TantrumChevronPair extends StatelessWidget {
  const TantrumChevronPair({super.key, required this.m, this.onUp, this.onDown});

  final TantrumMetrics m;
  final VoidCallback? onUp;
  final VoidCallback? onDown;

  @override
  Widget build(BuildContext context) {
    final w = 26 * m.scale;
    final h = 13 * m.scale;
    return Column(
      children: [
        _chevron(w, h, Icons.keyboard_arrow_up_rounded, onUp),
        SizedBox(height: 3 * m.scale),
        _chevron(w, h, Icons.keyboard_arrow_down_rounded, onDown),
      ],
    );
  }

  Widget _chevron(double w, double h, IconData icon, VoidCallback? tap) {
    return GestureDetector(
      onTap: tap,
      child: Container(
        width: w,
        height: h,
        decoration: BoxDecoration(
          color: Colors.white,
          borderRadius: BorderRadius.circular(4 * m.scale),
          boxShadow: [
            BoxShadow(color: TantrumTokens.knobShadow, blurRadius: 2 * m.scale, offset: const Offset(0, 1)),
          ],
        ),
        child: Icon(icon, size: h * 0.95, color: TantrumTokens.ink),
      ),
    );
  }
}

/// 读数项：小标签 + 红色数值（图1 的 PHASE AMOUNT 21）
class TantrumReadout extends StatelessWidget {
  const TantrumReadout({super.key, required this.label, required this.value, required this.m});

  final String label;
  final String value;
  final TantrumMetrics m;

  @override
  Widget build(BuildContext context) {
    return Row(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(label, style: TantrumTokens.caption(m.scale)),
        SizedBox(width: 5 * m.scale),
        Text(value, style: TantrumTokens.readout(m.scale)),
      ],
    );
  }
}
