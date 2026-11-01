
import 'package:flutter/material.dart';

import 'app_model.dart';
import 'models.dart';
import 'theme.dart';

class Panel extends StatelessWidget {
  const Panel({
    super.key,
    required this.child,
    this.padding = const EdgeInsets.all(20),
    this.accent,
    this.radius = 18,
  });

  final Widget child;
  final EdgeInsetsGeometry padding;
  final Color? accent;
  final double radius;

  @override
  Widget build(BuildContext context) {
    final borderColor = accent?.withValues(alpha: 0.28) ?? AppColors.borderSoft;
    return Container(
      decoration: BoxDecoration(
        gradient: const LinearGradient(
          colors: [AppColors.surface, AppColors.surface2],
          begin: Alignment.topLeft,
          end: Alignment.bottomRight,
        ),
        borderRadius: BorderRadius.circular(radius),
        border: Border.all(color: borderColor),
        boxShadow: [
          BoxShadow(
            color: Colors.black.withValues(alpha: 0.26),
            blurRadius: 28,
            offset: const Offset(0, 14),
          ),
        ],
      ),
      child: Padding(padding: padding, child: child),
    );
  }
}

class PanelHeader extends StatelessWidget {
  const PanelHeader({
    super.key,
    required this.title,
    this.subtitle,
    this.trailing,
    this.icon,
  });

  final String title;
  final String? subtitle;
  final Widget? trailing;
  final IconData? icon;

  @override
  Widget build(BuildContext context) {
    return Row(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        if (icon != null) ...[
          Container(
            width: 34,
            height: 34,
            decoration: BoxDecoration(
              color: AppColors.accent.withValues(alpha: 0.1),
              borderRadius: BorderRadius.circular(10),
              border: Border.all(color: AppColors.accent.withValues(alpha: 0.2)),
            ),
            child: Icon(icon, size: 18, color: AppColors.accent),
          ),
          const SizedBox(width: 12),
        ],
        Expanded(
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Text(title, style: Theme.of(context).textTheme.titleMedium),
              if (subtitle != null) ...[
                const SizedBox(height: 3),
                Text(subtitle!, style: Theme.of(context).textTheme.bodySmall),
              ],
            ],
          ),
        ),
        ?trailing,
      ],
    );
  }
}

class StatusPill extends StatelessWidget {
  const StatusPill({
    super.key,
    required this.label,
    required this.color,
    this.dot = true,
  });

  final String label;
  final Color color;
  final bool dot;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 10, vertical: 6),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.1),
        borderRadius: BorderRadius.circular(999),
        border: Border.all(color: color.withValues(alpha: 0.32)),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          if (dot) ...[
            Container(
              width: 6,
              height: 6,
              decoration: BoxDecoration(
                color: color,
                shape: BoxShape.circle,
                boxShadow: [
                  BoxShadow(color: color.withValues(alpha: 0.6), blurRadius: 6),
                ],
              ),
            ),
            const SizedBox(width: 7),
          ],
          Text(
            label,
            style: TextStyle(
              color: color,
              fontSize: 11,
              fontWeight: FontWeight.w700,
              letterSpacing: 0.3,
            ),
          ),
        ],
      ),
    );
  }
}

class CodecBadge extends StatelessWidget {
  const CodecBadge({super.key, required this.codec});

  final String codec;

  @override
  Widget build(BuildContext context) {
    final isOpus = codec.toLowerCase() == 'opus';
    final color = isOpus ? AppColors.violet : AppColors.accent2;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 3),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.1),
        borderRadius: BorderRadius.circular(6),
        border: Border.all(color: color.withValues(alpha: 0.3)),
      ),
      child: Text(
        codec.toUpperCase(),
        style: TextStyle(
          color: color,
          fontSize: 9.5,
          fontWeight: FontWeight.w800,
          letterSpacing: 0.7,
        ),
      ),
    );
  }
}

class StatChip extends StatelessWidget {
  const StatChip({
    super.key,
    required this.label,
    required this.value,
    this.color,
    this.mono = true,
  });

  final String label;
  final String value;
  final Color? color;
  final bool mono;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 6),
      decoration: BoxDecoration(
        color: AppColors.bg2.withValues(alpha: 0.7),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: AppColors.borderSoft),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Text(
            label,
            style: const TextStyle(color: AppColors.textFaint, fontSize: 10.5),
          ),
          const SizedBox(width: 7),
          Text(
            value,
            style: (mono ? AppText.mono : const TextStyle()).copyWith(
              color: color ?? AppColors.textDim,
              fontSize: 11,
              fontWeight: FontWeight.w600,
            ),
          ),
        ],
      ),
    );
  }
}

class Metric extends StatelessWidget {
  const Metric({
    super.key,
    required this.label,
    required this.value,
    this.unit,
    this.color,
    this.large = false,
  });

  final String label;
  final String value;
  final String? unit;
  final Color? color;
  final bool large;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(
          label,
          style: const TextStyle(
            color: AppColors.textFaint,
            fontSize: 10.5,
            fontWeight: FontWeight.w600,
            letterSpacing: 0.4,
          ),
        ),
        const SizedBox(height: 5),
        Row(
          crossAxisAlignment: CrossAxisAlignment.baseline,
          textBaseline: TextBaseline.alphabetic,
          children: [
            Text(
              value,
              style: AppText.mono.copyWith(
                color: color ?? AppColors.text,
                fontSize: large ? 24 : 18,
                fontWeight: FontWeight.w700,
                letterSpacing: -0.4,
              ),
            ),
            if (unit != null) ...[
              const SizedBox(width: 4),
              Text(
                unit!,
                style: TextStyle(
                  color: color ?? AppColors.textDim,
                  fontSize: 10.5,
                  fontWeight: FontWeight.w500,
                ),
              ),
            ],
          ],
        ),
      ],
    );
  }
}

class LevelMeter extends StatelessWidget {
  const LevelMeter({
    super.key,
    required this.rmsDb,
    required this.peakDb,
    this.peakHoldDb,
    this.height = 10,
    this.showValue = true,
  });

  final double rmsDb;
  final double peakDb;
  final double? peakHoldDb;
  final double height;
  final bool showValue;

  @override
  Widget build(BuildContext context) {
    final rms = dbToFraction(rmsDb);
    final peak = dbToFraction(peakDb);
    final hold = dbToFraction(peakHoldDb ?? peakDb);
    return Row(
      children: [
        Expanded(
          child: SizedBox(
            height: height,
            child: CustomPaint(
              painter: _MeterPainter(rms: rms, peak: peak, hold: hold),
            ),
          ),
        ),
        if (showValue) ...[
          const SizedBox(width: 10),
          SizedBox(
            width: 56,
            child: Text(
              '${formatDb(rmsDb)} dB',
              textAlign: TextAlign.right,
              style: AppText.mono.copyWith(
                fontSize: 10.5,
                color: AppColors.textDim,
              ),
            ),
          ),
        ],
      ],
    );
  }
}

double dbToFraction(double db) => ((db.clamp(-60.0, 0.0) + 60.0) / 60.0);

class _MeterPainter extends CustomPainter {
  _MeterPainter({required this.rms, required this.peak, required this.hold});

  final double rms;
  final double peak;
  final double hold;

  @override
  void paint(Canvas canvas, Size size) {
    final radius = Radius.circular(size.height / 2);
    final track = RRect.fromRectAndRadius(Offset.zero & size, radius);
    canvas.drawRRect(track, Paint()..color = AppColors.bg2);
    canvas.drawRRect(
      track,
      Paint()
        ..style = PaintingStyle.stroke
        ..strokeWidth = 1
        ..color = AppColors.borderSoft,
    );

    if (rms > 0.002) {
      canvas.save();
      canvas.clipRRect(track);
      final shader = const LinearGradient(
        colors: [
          AppColors.green,
          AppColors.accent,
          AppColors.amber,
          AppColors.rose,
        ],
        stops: [0.0, 0.55, 0.82, 1.0],
      ).createShader(Rect.fromLTWH(0, 0, size.width, size.height));
      canvas.drawRect(
        Rect.fromLTWH(0, 0, size.width * rms, size.height),
        Paint()..shader = shader,
      );
      canvas.drawRect(
        Rect.fromLTWH(0, 0, size.width * rms, 1),
        Paint()..color = Colors.white.withValues(alpha: 0.2),
      );
      canvas.restore();
    }

    if (hold > 0.002) {
      final x = (size.width * hold).clamp(1.0, size.width - 1);
      canvas.drawLine(
        Offset(x, 1),
        Offset(x, size.height - 1),
        Paint()
          ..color = AppColors.text.withValues(alpha: 0.85)
          ..strokeWidth = 1.6,
      );
    }
    if (peak > 0.002) {
      final x = (size.width * peak).clamp(1.0, size.width - 1);
      canvas.drawLine(
        Offset(x, size.height * 0.2),
        Offset(x, size.height * 0.8),
        Paint()
          ..color = Colors.white.withValues(alpha: 0.45)
          ..strokeWidth = 1,
      );
    }
  }

  @override
  bool shouldRepaint(covariant _MeterPainter old) =>
      old.rms != rms || old.peak != peak || old.hold != hold;
}

class BufferBar extends StatelessWidget {
  const BufferBar({super.key, required this.bufferMs, required this.targetMs});

  final double bufferMs;
  final double targetMs;

  @override
  Widget build(BuildContext context) {
    final target = targetMs <= 0 ? 30.0 : targetMs;
    final ratio = (bufferMs / target).clamp(0.0, 2.0);
    final healthy = ratio >= 0.5 && ratio <= 1.5;
    final color = healthy ? AppColors.green : AppColors.amber;
    return SizedBox(
      width: 78,
      height: 4,
      child: Stack(
        children: [
          Container(
            decoration: BoxDecoration(
              color: AppColors.bg2,
              borderRadius: BorderRadius.circular(2),
            ),
          ),
          FractionallySizedBox(
            widthFactor: (ratio / 2.0).clamp(0.0, 1.0),
            child: Container(
              decoration: BoxDecoration(
                color: color.withValues(alpha: 0.8),
                borderRadius: BorderRadius.circular(2),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class EmptyState extends StatelessWidget {
  const EmptyState({
    super.key,
    required this.icon,
    required this.title,
    required this.message,
    this.action,
    this.compact = false,
  });

  final IconData icon;
  final String title;
  final String message;
  final Widget? action;
  final bool compact;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: EdgeInsets.symmetric(vertical: compact ? 24 : 44),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Container(
            width: compact ? 52 : 64,
            height: compact ? 52 : 64,
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              color: AppColors.accent.withValues(alpha: 0.07),
              border: Border.all(
                color: AppColors.accent.withValues(alpha: 0.16),
              ),
            ),
            child: Icon(
              icon,
              size: compact ? 24 : 30,
              color: AppColors.accent.withValues(alpha: 0.8),
            ),
          ),
          const SizedBox(height: 16),
          Text(
            title,
            style: Theme.of(context).textTheme.titleMedium,
            textAlign: TextAlign.center,
          ),
          const SizedBox(height: 6),
          ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 420),
            child: Text(
              message,
              textAlign: TextAlign.center,
              style: Theme.of(context).textTheme.bodySmall,
            ),
          ),
          if (action != null) ...[const SizedBox(height: 18), action!],
        ],
      ),
    );
  }
}

class LabeledField extends StatelessWidget {
  const LabeledField({
    super.key,
    required this.label,
    required this.child,
    this.width,
    this.hint,
  });

  final String label;
  final Widget child;
  final double? width;
  final String? hint;

  @override
  Widget build(BuildContext context) {
    final content = Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Text(
              label,
              style: const TextStyle(
                color: AppColors.textFaint,
                fontSize: 10.5,
                fontWeight: FontWeight.w700,
                letterSpacing: 0.4,
              ),
            ),
            if (hint != null) ...[
              const SizedBox(width: 6),
              Expanded(
                child: Text(
                  hint!,
                  overflow: TextOverflow.ellipsis,
                  style: const TextStyle(
                    color: AppColors.textFaint,
                    fontSize: 10,
                  ),
                ),
              ),
            ],
          ],
        ),
        const SizedBox(height: 7),
        child,
      ],
    );
    return width == null ? content : SizedBox(width: width, child: content);
  }
}

class SectionLabel extends StatelessWidget {
  const SectionLabel(this.text, {super.key, this.trailing});

  final String text;
  final Widget? trailing;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.only(bottom: 12),
      child: Row(
        children: [
          Text(
            text,
            style: const TextStyle(
              color: AppColors.textDim,
              fontSize: 11,
              fontWeight: FontWeight.w700,
              letterSpacing: 1.1,
            ),
          ),
          const SizedBox(width: 10),
          Expanded(child: Container(height: 1, color: AppColors.borderSoft)),
          if (trailing != null) ...[const SizedBox(width: 10), trailing!],
        ],
      ),
    );
  }
}

/// Pulsing three-dot busy indicator.
///
/// Box size is clamped to 20x20: on this Flutter/Windows build, a button icon
/// slot rebuilt to a smaller box crashes natively in flutter_windows.dll
/// (0xC0000005 while the engine walks an id table after the relayout).
/// Deliberately avoids [CircularProgressIndicator], which also triggered the
/// fault at its default 16x16 footprint.
class AppSpinner extends StatefulWidget {
  const AppSpinner({super.key, this.size = 16, this.color});

  final double size;
  final Color? color;

  @override
  State<AppSpinner> createState() => _AppSpinnerState();
}

class _AppSpinnerState extends State<AppSpinner>
    with SingleTickerProviderStateMixin {
  late final AnimationController _controller = AnimationController(
    vsync: this,
    duration: const Duration(milliseconds: 1000),
  )..repeat();

  @override
  void dispose() {
    _controller.dispose();
    super.dispose();
  }

  double _phase(int index) {
    final t = (_controller.value * 3 - index) % 1.0;
    return t < 0 ? t + 1.0 : t;
  }

  @override
  Widget build(BuildContext context) {
    final color = widget.color ?? AppColors.accent;
    final box = widget.size < 20 ? 20.0 : widget.size;
    final dot = box * 0.22;
    return AnimatedBuilder(
      animation: _controller,
      builder: (context, _) {
        return SizedBox(
          width: box,
          height: box,
          child: Row(
            mainAxisAlignment: MainAxisAlignment.spaceBetween,
            children: [
              for (var i = 0; i < 3; i++)
                Opacity(
                  opacity: 0.25 + 0.75 * _phase(i),
                  child: Container(
                    width: dot,
                    height: dot,
                    decoration: BoxDecoration(
                      color: color,
                      shape: BoxShape.circle,
                    ),
                  ),
                ),
            ],
          ),
        );
      },
    );
  }
}

/// One incoming stream: label, level meter, gain, health statistics.
class StreamCard extends StatefulWidget {
  const StreamCard({super.key, required this.model, required this.stream});

  final AppModel model;
  final StreamState stream;

  @override
  State<StreamCard> createState() => _StreamCardState();
}

class _StreamCardState extends State<StreamCard> {
  late double _gain;
  bool _dragging = false;

  static const _accents = [
    AppColors.accent,
    AppColors.accent2,
    AppColors.violet,
    AppColors.amber,
    AppColors.green,
    AppColors.rose,
  ];

  @override
  void initState() {
    super.initState();
    _gain = widget.stream.gainDb;
  }

  @override
  void didUpdateWidget(covariant StreamCard oldWidget) {
    super.didUpdateWidget(oldWidget);
    if (!_dragging) _gain = widget.stream.gainDb;
  }

  @override
  Widget build(BuildContext context) {
    final stream = widget.stream;
    final accent = _accents[stream.id.abs() % _accents.length];
    final health = stream.underruns == 0 && stream.bufferResets == 0;
    final displayName = widget.model.mesh?.streamName(stream) ?? stream.label;

    return Panel(
      padding: const EdgeInsets.fromLTRB(18, 16, 18, 15),
      child: Row(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Container(
            width: 3,
            height: 76,
            margin: const EdgeInsets.only(right: 14, top: 2),
            decoration: BoxDecoration(
              gradient: LinearGradient(
                begin: Alignment.topCenter,
                end: Alignment.bottomCenter,
                colors: [accent, accent.withValues(alpha: 0.12)],
              ),
              borderRadius: BorderRadius.circular(4),
            ),
          ),
          Expanded(
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  children: [
                    ConstrainedBox(
                      constraints: const BoxConstraints(maxWidth: 250),
                      child: Text(
                        displayName,
                        overflow: TextOverflow.ellipsis,
                        style: const TextStyle(
                          color: AppColors.text,
                          fontSize: 13,
                          fontWeight: FontWeight.w700,
                        ),
                      ),
                    ),
                    const SizedBox(width: 9),
                    CodecBadge(codec: stream.codec),
                    const SizedBox(width: 9),
                    StatusPill(
                      label: stream.active ? '在线' : '空闲',
                      color: stream.active ? AppColors.green : AppColors.textFaint,
                    ),
                    const SizedBox(width: 8),
                    StatusPill(
                      label: switch (stream.path) {
                        'bt' => '蓝牙',
                        'local' => '本地',
                        _ => 'Wi-Fi',
                      },
                      color: stream.path == 'bt'
                          ? AppColors.accent2
                          : AppColors.textDim,
                      dot: false,
                    ),
                    const SizedBox(width: 8),
                    if (!health)
                      StatusPill(
                        label: '需关注',
                        color: AppColors.amber,
                        dot: false,
                      ),
                    const Spacer(),
                    Tooltip(
                      message: stream.muted ? '取消静音' : '静音',
                      child: IconButton(
                        onPressed: () => widget.model.setStreamMuted(
                          stream.id,
                          !stream.muted,
                        ),
                        icon: Icon(
                          stream.muted
                              ? Icons.volume_off_rounded
                              : Icons.volume_up_rounded,
                          size: 19,
                          color: stream.muted
                              ? AppColors.rose
                              : AppColors.textDim,
                        ),
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 8),
                Row(
                  children: [
                    Expanded(
                      child: LevelMeter(
                        rmsDb: stream.rmsDb,
                        peakDb: stream.peakDb,
                        peakHoldDb: stream.peakHoldDb,
                        height: 9,
                      ),
                    ),
                    const SizedBox(width: 18),
                    SizedBox(
                      width: 190,
                      child: Row(
                        children: [
                          const Text(
                            '增益',
                            style: TextStyle(
                              color: AppColors.textFaint,
                              fontSize: 10.5,
                            ),
                          ),
                          Expanded(
                            child: Slider(
                              min: -30,
                              max: 12,
                              divisions: 84,
                              value: _gain.clamp(-30, 12),
                              onChanged: (value) {
                                setState(() {
                                  _dragging = true;
                                  _gain = value;
                                });
                                widget.model.setStreamGain(stream.id, value);
                              },
                              onChangeEnd: (value) {
                                widget.model.setStreamGain(stream.id, value);
                                setState(() {
                                  _dragging = false;
                                  _gain = value;
                                });
                              },
                            ),
                          ),
                          SizedBox(
                            width: 52,
                            child: Text(
                              '${_gain >= 0 ? '+' : ''}${_gain.toStringAsFixed(1)}',
                              textAlign: TextAlign.right,
                              style: AppText.mono.copyWith(
                                color: AppColors.textDim,
                                fontSize: 10.5,
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
                const SizedBox(height: 10),
                Wrap(
                  spacing: 7,
                  runSpacing: 7,
                  crossAxisAlignment: WrapCrossAlignment.center,
                  children: [
                    _BufferChip(
                      bufferMs: stream.bufferMs,
                      targetMs: stream.targetMs,
                    ),
                    StatChip(
                      label: 'ratio',
                      value: stream.ratio.toStringAsFixed(5),
                    ),
                    StatChip(
                      label: '包',
                      value: formatCount(stream.packets),
                    ),
                    StatChip(
                      label: '丢帧',
                      value: formatCount(stream.lostFrames),
                      color:
                          stream.lostFrames > 0 ? AppColors.rose : AppColors.textDim,
                    ),
                    StatChip(
                      label: '欠载',
                      value: '${stream.underruns}',
                      color: stream.underruns > 0
                          ? AppColors.amber
                          : AppColors.textDim,
                    ),
                    StatChip(
                      label: '回落',
                      value: '${stream.bufferResets}',
                      color: stream.bufferResets > 0
                          ? AppColors.violet
                          : AppColors.textDim,
                    ),
                    if (stream.plcFrames > 0)
                      StatChip(
                        label: 'PLC',
                        value: formatCount(stream.plcFrames),
                        color: AppColors.violet,
                      ),
                    StatChip(
                      label: '最后包',
                      value: '${stream.ageMs} ms',
                    ),
                  ],
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

class _BufferChip extends StatelessWidget {
  const _BufferChip({required this.bufferMs, required this.targetMs});

  final double bufferMs;
  final double targetMs;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 9, vertical: 6),
      decoration: BoxDecoration(
        color: AppColors.bg2.withValues(alpha: 0.7),
        borderRadius: BorderRadius.circular(8),
        border: Border.all(color: AppColors.borderSoft),
      ),
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          const Text(
            '缓冲',
            style: TextStyle(color: AppColors.textFaint, fontSize: 10.5),
          ),
          const SizedBox(width: 7),
          BufferBar(bufferMs: bufferMs, targetMs: targetMs),
          const SizedBox(width: 7),
          Text(
            '${bufferMs.toStringAsFixed(1)} / ${targetMs.toStringAsFixed(0)} ms',
            style: AppText.mono.copyWith(
              color: AppColors.textDim,
              fontSize: 10.5,
              fontWeight: FontWeight.w600,
            ),
          ),
        ],
      ),
    );
  }
}
