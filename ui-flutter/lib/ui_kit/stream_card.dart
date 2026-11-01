import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'theme.dart';

/// 桌面远端流卡片：只保留增益旋钮、当前电平窗、路径与丢帧/欠载。
///
/// 顶部三枚控件依次是连接状态、信号质量灯与一键静音。
class StreamCard extends StatelessWidget {
  const StreamCard({
    super.key,
    required this.channel,
    required this.palette,
    required this.size,
  });

  final ModuleChannel channel;
  final MetalPalette palette;
  final Size size;

  @override
  Widget build(BuildContext context) {
    return ListenableBuilder(
      listenable: channel,
      builder: (context, _) {
        final p = palette;
        return Opacity(
          opacity: channel.on && !channel.mute ? 1 : .62,
          child: NeuRaised(
            palette: p,
            size: size,
            radius: 14,
            padding: const EdgeInsets.fromLTRB(14, 12, 14, 12),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                SizedBox(
                  height: 20,
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          channel.title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: NeuType.chrome(16.5, p.inkStrong, tracking: 1.2, w: FontWeight.w700),
                        ),
                      ),
                      Tooltip(
                        message: '连接状态',
                        child: NeuLed(palette: p, color: p.ledOn, on: channel.on, size: 8),
                      ),
                      const SizedBox(width: 7),
                      Tooltip(
                        message: '信号质量',
                        child: NeuLed(
                          palette: p,
                          color: _healthColor(p, channel.health),
                          on: channel.health < 3,
                          size: 8,
                        ),
                      ),
                      const SizedBox(width: 9),
                      _MuteButton(
                        palette: p,
                        muted: channel.mute,
                        onTap: channel.toggleMute,
                      ),
                    ],
                  ),
                ),
                const SizedBox(height: 8),
                Center(
                  child: NeuKnob(
                    palette: p,
                    size: 112,
                    value: channel.amount,
                    range: channel.amountRange,
                    accentColor: channel.accentFor(p),
                    onChanged: channel.setAmount,
                  ),
                ),
                const SizedBox(height: 2),
                Center(
                  child: SizedBox(
                    width: 112,
                    child: Row(
                      mainAxisAlignment: MainAxisAlignment.spaceBetween,
                      children: [
                        Text(
                          channel.amountRange.min.toInt().toString(),
                          style: NeuType.chrome(10, p.inkFaint, tracking: .6),
                        ),
                        Text(
                          channel.amountRange.min < 0
                              ? '+${channel.amountRange.max.toInt()}'
                              : channel.amountRange.max.toInt().toString(),
                          style: NeuType.chrome(10, p.inkFaint, tracking: .6),
                        ),
                      ],
                    ),
                  ),
                ),
                const SizedBox(height: 6),
                Center(
                  child: NeuValueField(
                    palette: p,
                    value: channel.amount,
                    range: channel.amountRange,
                    onChanged: channel.setAmount,
                    width: 96,
                    height: 26,
                    unit: channel.amountUnit,
                    accent: channel.accentFor(p),
                    fontSize: 13,
                  ),
                ),
                const Spacer(),
                _LevelWindow(palette: p, db: channel.rmsDb, accent: channel.accentFor(p)),
                const SizedBox(height: 8),
                _Facts(palette: p, channel: channel),
              ],
            ),
          ),
        );
      },
    );
  }

  static Color _healthColor(MetalPalette p, int health) => switch (health) {
        0 => p.ledOn,
        1 => p.orange,
        2 => p.salmon,
        _ => p.ledDim,
      };
}

/// 当前电平：数值 + 横向电平条。
class _LevelWindow extends StatelessWidget {
  const _LevelWindow({required this.palette, required this.db, required this.accent});

  final MetalPalette palette;
  final double db;
  final Color accent;

  @override
  Widget build(BuildContext context) {
    final norm = ((db + 60) / 60).clamp(0.0, 1.0);
    final barColor = norm > .94
        ? palette.salmon
        : norm > .8
            ? palette.orange
            : accent;
    return SizedBox(
      height: 50,
      child: NeuWell(
        radius: 9,
        palette: palette,
        deep: true,
        padding: const EdgeInsets.fromLTRB(9, 6, 9, 7),
        child: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Row(
              children: [
                Text('当前电平', style: NeuType.chrome(9.5, palette.inkFaint, tracking: .6)),
                const Spacer(),
                Text(
                  '${db.toStringAsFixed(1)} dB',
                  style: NeuType.numeric(13, barColor),
                ),
              ],
            ),
            const Spacer(),
            SizedBox(
              height: 6,
              child: LayoutBuilder(
                builder: (context, box) => Stack(
                  children: [
                    Positioned.fill(
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          color: palette.bevel.withValues(alpha: .16),
                          borderRadius: BorderRadius.circular(3),
                        ),
                      ),
                    ),
                    Positioned(
                      left: 0,
                      top: 0,
                      bottom: 0,
                      width: box.maxWidth * norm,
                      child: DecoratedBox(
                        decoration: BoxDecoration(
                          gradient: LinearGradient(colors: [barColor.withValues(alpha: .55), barColor]),
                          borderRadius: BorderRadius.circular(3),
                        ),
                      ),
                    ),
                  ],
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 路径 / 丢帧 / 欠载。
class _Facts extends StatelessWidget {
  const _Facts({required this.palette, required this.channel});

  final MetalPalette palette;
  final ModuleChannel channel;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 56,
      child: NeuWell(
        radius: 9,
        palette: palette,
        tint: palette.wellTint,
        padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 5),
        child: Row(
          children: [
            Expanded(child: _cell('路径', channel.pathText)),
            _divider(),
            Expanded(child: _cell('丢帧', '${channel.lostFrames}')),
            _divider(),
            Expanded(child: _cell('欠载', '${channel.underruns}')),
          ],
        ),
      ),
    );
  }

  Widget _divider() => Container(
        width: 1,
        height: 24,
        color: palette.bevel.withValues(alpha: .35),
      );

  Widget _cell(String label, String value) {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      crossAxisAlignment: CrossAxisAlignment.start,
      children: [
        Text(label, style: NeuType.chrome(9.5, palette.inkFaint, tracking: .8)),
        const SizedBox(height: 2),
        Text(
          value,
          maxLines: 1,
          overflow: TextOverflow.ellipsis,
          style: NeuType.numeric(11.5, palette.ink),
        ),
      ],
    );
  }
}


/// 一键静音：圆形金属键 + 扬声器图标，静音时整键变红。
class _MuteButton extends StatelessWidget {
  const _MuteButton({required this.palette, required this.muted, required this.onTap});

  final MetalPalette palette;
  final bool muted;
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) {
    final p = palette;
    return Tooltip(
      message: muted ? '取消静音' : '静音',
      child: GestureDetector(
        behavior: HitTestBehavior.opaque,
        onTap: onTap,
        child: SizedBox(
          width: 20,
          height: 20,
          child: DecoratedBox(
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              gradient: LinearGradient(
                begin: Alignment.topCenter,
                end: Alignment.bottomCenter,
                colors: muted
                    ? [p.salmon.withValues(alpha: .95), p.salmon.withValues(alpha: .7)]
                    : [p.moduleTop, p.moduleBottom],
              ),
              boxShadow: [
                BoxShadow(
                  color: muted ? p.salmon.withValues(alpha: .35) : p.bevel.withValues(alpha: .22),
                  offset: const Offset(0, 1),
                  blurRadius: muted ? 6 : 2,
                ),
              ],
            ),
            child: Icon(
              muted ? Icons.volume_off_rounded : Icons.volume_up_rounded,
              size: 12,
              color: muted ? Colors.white : p.ink,
            ),
          ),
        ),
      ),
    );
  }
}
