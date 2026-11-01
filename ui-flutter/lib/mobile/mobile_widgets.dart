import 'package:flutter/material.dart';

import '../app_model.dart';
import '../models.dart';
import '../theme.dart';
import '../widgets.dart';

/// Shared building blocks for the mobile UI: large touch targets, one
/// column, cards on the dark background.

class MobileCard extends StatelessWidget {
  const MobileCard({
    super.key,
    required this.children,
    this.icon,
    this.title,
    this.subtitle,
    this.accent,
  });

  final List<Widget> children;
  final IconData? icon;
  final String? title;
  final String? subtitle;
  final Color? accent;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.all(18),
      decoration: BoxDecoration(
        color: AppColors.surface,
        borderRadius: BorderRadius.circular(20),
        border: Border.all(
          color: accent?.withValues(alpha: 0.35) ?? AppColors.borderSoft,
        ),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          if (title != null) ...[
            Row(
              children: [
                if (icon != null) ...[
                  Container(
                    width: 38,
                    height: 38,
                    decoration: BoxDecoration(
                      color: AppColors.raised,
                      borderRadius: BorderRadius.circular(12),
                      border: Border.all(color: AppColors.borderSoft),
                    ),
                    child: Icon(icon, size: 20, color: accent ?? AppColors.accent),
                  ),
                  const SizedBox(width: 12),
                ],
                Expanded(
                  child: Text(
                    title!,
                    style: const TextStyle(
                      color: AppColors.text,
                      fontSize: 16,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                ),
              ],
            ),
            if (subtitle != null) ...[
              const SizedBox(height: 6),
              Text(
                subtitle!,
                style: const TextStyle(
                  color: AppColors.textFaint,
                  fontSize: 12,
                ),
              ),
            ],
            const SizedBox(height: 16),
          ],
          ...children,
        ],
      ),
    );
  }
}

class MobileSegmented<T> extends StatelessWidget {
  const MobileSegmented({
    super.key,
    required this.label,
    required this.value,
    required this.items,
    required this.onChanged,
  });

  final String label;
  final T value;
  final List<({T value, String label, IconData icon})> items;
  final ValueChanged<T> onChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        MobileFieldLabel(label),
        const SizedBox(height: 8),
        Row(
          children: [
            for (final (index, item) in items.indexed) ...[
              if (index > 0) const SizedBox(width: 8),
              Expanded(
                child: _Segment(
                  item: item,
                  selected: item.value == value,
                  onTap: () => onChanged(item.value),
                ),
              ),
            ],
          ],
        ),
      ],
    );
  }
}

class _Segment extends StatelessWidget {
  const _Segment({
    required this.item,
    required this.selected,
    required this.onTap,
  });

  final ({dynamic value, String label, IconData icon}) item;
  final bool selected;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final color = selected ? AppColors.accent : AppColors.textDim;
    return Material(
      color: selected ? AppColors.accent.withValues(alpha: 0.12) : AppColors.bg2,
      borderRadius: BorderRadius.circular(14),
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(14),
        child: Container(
          height: 48,
          decoration: BoxDecoration(
            borderRadius: BorderRadius.circular(14),
            border: Border.all(
              color: selected ? AppColors.accent.withValues(alpha: 0.5) : AppColors.border,
            ),
          ),
          child: Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Icon(item.icon, size: 17, color: color),
              const SizedBox(width: 6),
              Text(
                item.label,
                style: TextStyle(
                  color: selected ? AppColors.text : color,
                  fontSize: 13,
                  fontWeight: FontWeight.w600,
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}

class MobileFieldLabel extends StatelessWidget {
  const MobileFieldLabel(this.label, {super.key});

  final String label;

  @override
  Widget build(BuildContext context) {
    return Text(
      label,
      style: const TextStyle(
        color: AppColors.textDim,
        fontSize: 12,
        fontWeight: FontWeight.w600,
      ),
    );
  }
}

class MobileTextField extends StatelessWidget {
  const MobileTextField({
    super.key,
    required this.controller,
    required this.label,
    this.keyboardType,
    this.enabled = true,
    this.hint,
  });

  final TextEditingController controller;
  final String label;
  final TextInputType? keyboardType;
  final bool enabled;
  final String? hint;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        MobileFieldLabel(label),
        const SizedBox(height: 8),
        TextField(
          controller: controller,
          enabled: enabled,
          keyboardType: keyboardType,
          style: AppText.mono.copyWith(color: AppColors.text, fontSize: 15),
          decoration: InputDecoration(
            filled: true,
            fillColor: AppColors.bg2,
            hintText: hint,
            hintStyle: const TextStyle(color: AppColors.textFaint),
            contentPadding:
                const EdgeInsets.symmetric(horizontal: 14, vertical: 14),
            enabledBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(14),
              borderSide: const BorderSide(color: AppColors.border),
            ),
            focusedBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(14),
              borderSide: BorderSide(color: AppColors.accent.withValues(alpha: 0.6)),
            ),
            disabledBorder: OutlineInputBorder(
              borderRadius: BorderRadius.circular(14),
              borderSide: const BorderSide(color: AppColors.borderSoft),
            ),
          ),
        ),
      ],
    );
  }
}

class MobileSwitchRow extends StatelessWidget {
  const MobileSwitchRow({
    super.key,
    required this.label,
    required this.value,
    required this.onChanged,
  });

  final String label;
  final bool value;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) {
    return Row(
      children: [
        Expanded(child: MobileFieldLabel(label)),
        Switch(
          value: value,
          onChanged: onChanged,
          activeThumbColor: AppColors.accent,
          activeTrackColor: AppColors.accent.withValues(alpha: 0.3),
          inactiveThumbColor: AppColors.textFaint,
          inactiveTrackColor: AppColors.bg2,
        ),
      ],
    );
  }
}

class MobilePrimaryButton extends StatelessWidget {
  const MobilePrimaryButton({
    super.key,
    required this.label,
    required this.onPressed,
    this.progress = false,
  });

  final String label;
  final VoidCallback? onPressed;
  final bool progress;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: double.infinity,
      height: 52,
      child: FilledButton(
        onPressed: onPressed,
        style: FilledButton.styleFrom(
          backgroundColor: AppColors.accent,
          foregroundColor: AppColors.ink,
          shape:
              RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
        ),
        child: progress
            ? const AppSpinner(size: 18, color: AppColors.ink)
            : Text(
                label,
                style: const TextStyle(
                  fontSize: 15,
                  fontWeight: FontWeight.w700,
                ),
              ),
      ),
    );
  }
}

class MobileDangerButton extends StatelessWidget {
  const MobileDangerButton({
    super.key,
    required this.label,
    required this.onPressed,
  });

  final String label;
  final VoidCallback? onPressed;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      width: double.infinity,
      height: 52,
      child: OutlinedButton(
        onPressed: onPressed,
        style: OutlinedButton.styleFrom(
          foregroundColor: AppColors.rose,
          side: BorderSide(color: AppColors.rose.withValues(alpha: 0.45)),
          shape:
              RoundedRectangleBorder(borderRadius: BorderRadius.circular(16)),
        ),
        child: Text(
          label,
          style: const TextStyle(fontSize: 15, fontWeight: FontWeight.w700),
        ),
      ),
    );
  }
}

class MobileStat extends StatelessWidget {
  const MobileStat({
    super.key,
    required this.label,
    required this.value,
    this.color,
  });

  final String label;
  final String value;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 14, vertical: 10),
      decoration: BoxDecoration(
        color: AppColors.bg2,
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: AppColors.borderSoft),
      ),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            label,
            style: const TextStyle(color: AppColors.textFaint, fontSize: 10.5),
          ),
          const SizedBox(height: 2),
          Text(
            value,
            style: AppText.mono.copyWith(
              color: color ?? AppColors.text,
              fontSize: 14,
              fontWeight: FontWeight.w700,
            ),
          ),
        ],
      ),
    );
  }
}

class MobileSlider extends StatelessWidget {
  const MobileSlider({
    super.key,
    required this.label,
    required this.value,
    required this.min,
    required this.max,
    required this.display,
    required this.onChanged,
  });

  final String label;
  final double value;
  final double min;
  final double max;
  final String display;
  final ValueChanged<double>? onChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Row(
          children: [
            Expanded(child: MobileFieldLabel(label)),
            Text(
              display,
              style: AppText.mono.copyWith(
                color: AppColors.accent,
                fontSize: 12,
                fontWeight: FontWeight.w700,
              ),
            ),
          ],
        ),
        SliderTheme(
          data: SliderTheme.of(context).copyWith(
            trackHeight: 4,
            overlayShape: const RoundSliderOverlayShape(overlayRadius: 22),
          ),
          child: Slider(
            min: min,
            max: max,
            value: value.clamp(min, max),
            onChanged: onChanged,
          ),
        ),
      ],
    );
  }
}

class MobileEmpty extends StatelessWidget {
  const MobileEmpty({
    super.key,
    required this.icon,
    required this.title,
    required this.message,
  });

  final IconData icon;
  final String title;
  final String message;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      padding: const EdgeInsets.all(28),
      decoration: BoxDecoration(
        color: AppColors.surface,
        borderRadius: BorderRadius.circular(20),
        border: Border.all(color: AppColors.borderSoft),
      ),
      child: Column(
        children: [
          Container(
            width: 54,
            height: 54,
            decoration: BoxDecoration(
              color: AppColors.raised,
              shape: BoxShape.circle,
            ),
            child: Icon(icon, color: AppColors.accent, size: 26),
          ),
          const SizedBox(height: 14),
          Text(
            title,
            style: const TextStyle(
              color: AppColors.text,
              fontSize: 14,
              fontWeight: FontWeight.w700,
            ),
          ),
          const SizedBox(height: 6),
          Text(
            message,
            textAlign: TextAlign.center,
            style: const TextStyle(
              color: AppColors.textFaint,
              fontSize: 12,
              height: 1.5,
            ),
          ),
        ],
      ),
    );
  }
}

class MobileNotice extends StatelessWidget {
  const MobileNotice({
    super.key,
    required this.message,
    required this.color,
    this.onDismiss,
  });

  final String message;
  final Color color;
  final VoidCallback? onDismiss;

  @override
  Widget build(BuildContext context) {
    return Container(
      width: double.infinity,
      margin: const EdgeInsets.only(bottom: 14),
      padding: const EdgeInsets.fromLTRB(14, 12, 8, 12),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.08),
        borderRadius: BorderRadius.circular(14),
        border: Border.all(color: color.withValues(alpha: 0.35)),
      ),
      child: Row(
        children: [
          Expanded(
            child: Text(
              message,
              style: TextStyle(
                color: color.withValues(alpha: 0.95),
                fontSize: 12,
                height: 1.4,
              ),
            ),
          ),
          if (onDismiss != null)
            IconButton(
              visualDensity: VisualDensity.compact,
              onPressed: onDismiss,
              icon: Icon(Icons.close_rounded, size: 16, color: color),
            ),
        ],
      ),
    );
  }
}

/// One connected stream: header, thin meter, gain slider + mute.
class StreamTile extends StatefulWidget {
  const StreamTile({super.key, required this.model, required this.stream});

  final AppModel model;
  final StreamState stream;

  @override
  State<StreamTile> createState() => _StreamTileState();
}

class _StreamTileState extends State<StreamTile> {
  bool _expanded = false;

  @override
  Widget build(BuildContext context) {
    final stream = widget.stream;
    final accent = stream.muted
        ? AppColors.rose
        : stream.active
            ? AppColors.accent
            : AppColors.textFaint;
    final streamName = widget.model.mesh?.streamName(stream) ?? stream.label;
    return AnimatedBuilder(
      animation: widget.model,
      builder: (context, _) {
        return Container(
          padding: const EdgeInsets.all(16),
          decoration: BoxDecoration(
            color: AppColors.surface,
            borderRadius: BorderRadius.circular(18),
            border: Border.all(color: AppColors.borderSoft),
          ),
          child: Column(
            crossAxisAlignment: CrossAxisAlignment.start,
            children: [
              Row(
                children: [
                  Container(
                    width: 4,
                    height: 34,
                    decoration: BoxDecoration(
                      color: accent,
                      borderRadius: BorderRadius.circular(2),
                    ),
                  ),
                  const SizedBox(width: 10),
                  Expanded(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        Text(
                          streamName,
                          overflow: TextOverflow.ellipsis,
                          style: const TextStyle(
                            color: AppColors.text,
                            fontSize: 13.5,
                            fontWeight: FontWeight.w700,
                          ),
                        ),
                        const SizedBox(height: 2),
                        Text(
                          stream.addr,
                          overflow: TextOverflow.ellipsis,
                          style: AppText.mono.copyWith(
                            color: AppColors.textFaint,
                            fontSize: 10.5,
                          ),
                        ),
                        const SizedBox(height: 3),
                        Row(
                          children: [
                            _MiniTag(stream.codec.toUpperCase()),
                            const SizedBox(width: 6),
                            _MiniTag(
                              stream.muted ? '已静音' : '在线',
                              color: accent,
                            ),
                          ],
                        ),
                      ],
                    ),
                  ),
                  IconButton(
                    onPressed: () => widget.model.setStreamMuted(
                      stream.id,
                      !stream.muted,
                    ),
                    icon: Icon(
                      stream.muted
                          ? Icons.volume_off_rounded
                          : Icons.volume_up_rounded,
                      color: stream.muted ? AppColors.rose : AppColors.textDim,
                      size: 22,
                    ),
                  ),
                  IconButton(
                    onPressed: () => setState(() => _expanded = !_expanded),
                    icon: Icon(
                      _expanded
                          ? Icons.expand_less_rounded
                          : Icons.expand_more_rounded,
                      color: AppColors.textFaint,
                      size: 22,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 12),
              LevelMeter(
                rmsDb: stream.rmsDb,
                peakDb: stream.peakDb,
                peakHoldDb: stream.peakHoldDb,
                height: 10,
                showValue: false,
              ),
              const SizedBox(height: 8),
              Row(
                children: [
                  Text(
                    '${formatDb(stream.rmsDb)} dB',
                    style: AppText.mono.copyWith(
                      color: AppColors.textDim,
                      fontSize: 11,
                    ),
                  ),
                  const Spacer(),
                  Text(
                    '${stream.bufferMs.toStringAsFixed(1)} / ${stream.targetMs.toStringAsFixed(0)} ms',
                    style: AppText.mono.copyWith(
                      color: AppColors.textFaint,
                      fontSize: 11,
                    ),
                  ),
                ],
              ),
              const SizedBox(height: 4),
              MobileSlider(
                label: '增益',
                value: stream.gainDb,
                min: -30,
                max: 12,
                display:
                    '${stream.gainDb >= 0 ? '+' : ''}${stream.gainDb.toStringAsFixed(1)} dB',
                onChanged: (value) =>
                    widget.model.setStreamGain(stream.id, value),
              ),
              if (_expanded) ...[
                const SizedBox(height: 4),
                Row(
                  children: [
                    Expanded(
                      child: MobileStat(
                        label: '收包',
                        value: formatCount(stream.packets),
                      ),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: MobileStat(
                        label: '丢帧',
                        value: formatCount(stream.lostFrames),
                        color: stream.lostFrames > 0 ? AppColors.amber : null,
                      ),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: MobileStat(
                        label: '欠载',
                        value: formatCount(stream.underruns),
                        color: stream.underruns > 0 ? AppColors.amber : null,
                      ),
                    ),
                    const SizedBox(width: 8),
                    Expanded(
                      child: MobileStat(
                        label: '漂移',
                        value: stream.ratio.toStringAsFixed(5),
                      ),
                    ),
                  ],
                ),
              ],
            ],
          ),
        );
      },
    );
  }
}

class _MiniTag extends StatelessWidget {
  const _MiniTag(this.label, {this.color});

  final String label;
  final Color? color;

  @override
  Widget build(BuildContext context) {
    final color = this.color ?? AppColors.accent;
    return Container(
      padding: const EdgeInsets.symmetric(horizontal: 7, vertical: 2),
      decoration: BoxDecoration(
        color: color.withValues(alpha: 0.1),
        borderRadius: BorderRadius.circular(6),
      ),
      child: Text(
        label,
        style: AppText.mono.copyWith(color: color, fontSize: 9.5),
      ),
    );
  }
}
