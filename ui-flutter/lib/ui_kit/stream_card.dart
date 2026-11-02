import 'package:flutter/material.dart';

import 'controls.dart';
import 'model.dart';
import 'theme.dart';

/// 桌面远端流卡片，对应参考图的 SUB 1 / SUB 2：
/// 只有一颗大增益旋钮，右侧是电平窗，窗下竖排路径/丢帧/欠载。
///
/// 右上角三枚依次是连接状态、信号质量与一键静音。
class StreamCard extends StatelessWidget {
  const StreamCard({
    super.key,
    required this.channel,
    required this.palette,
    required this.size,
  });

  static const Size design = Size(242, 268);

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
            radius: 16,
            padding: const EdgeInsets.fromLTRB(16, 14, 16, 14),
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                SizedBox(
                  height: 22,
                  child: Row(
                    children: [
                      Expanded(
                        child: Text(
                          channel.title,
                          maxLines: 1,
                          overflow: TextOverflow.ellipsis,
                          style: NeuType.chrome(
                            16.5,
                            p.inkStrong,
                            tracking: 1.2,
                            w: FontWeight.w700,
                          ),
                        ),
                      ),
                      Tooltip(
                        message: '连接状态',
                        child: NeuLed(
                          palette: p,
                          color: p.ledOn,
                          on: channel.on,
                          size: 8,
                        ),
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
                Expanded(
                  child: Row(
                    children: [
                      SizedBox(
                        width: 112,
                        child: Column(
                          mainAxisAlignment: MainAxisAlignment.center,
                          children: [
                            NeuKnob(
                              palette: p,
                              size: 112,
                              value: channel.amount,
                              range: channel.amountRange,
                              accentColor: channel.accentFor(p),
                              onChanged: channel.setAmount,
                            ),
                            const SizedBox(height: 2),
                            SizedBox(
                              width: 108,
                              child: Row(
                                mainAxisAlignment:
                                    MainAxisAlignment.spaceBetween,
                                children: [
                                  Text(
                                    channel.amountRange.min.toInt().toString(),
                                    style: NeuType.chrome(
                                      9.5,
                                      p.inkFaint,
                                      tracking: .4,
                                    ),
                                  ),
                                  Text(
                                    channel.amountRange.min < 0
                                        ? '+${channel.amountRange.max.toInt()}'
                                        : channel.amountRange.max
                                              .toInt()
                                              .toString(),
                                    style: NeuType.chrome(
                                      9.5,
                                      p.inkFaint,
                                      tracking: .4,
                                    ),
                                  ),
                                ],
                              ),
                            ),
                            const SizedBox(height: 4),
                            Text(
                              '增益',
                              style: NeuType.chrome(
                                11,
                                p.ink,
                                tracking: 1.6,
                                w: FontWeight.w700,
                              ),
                            ),
                          ],
                        ),
                      ),
                      const SizedBox(width: 6),
                      // 桌面卡窄，电平窗与事实条上下排；手机卡宽，改成左右排。
                      Expanded(
                        child: LayoutBuilder(
                          builder: (context, box) {
                            final window = _LevelWindow(
                              palette: p,
                              db: channel.rmsDb,
                              accent: channel.accentFor(p),
                              bufferMs: channel.bufferMs,
                              bufferMaxMs: channel.bufferMaxMs,
                            );
                            final facts = _Facts(palette: p, channel: channel);
                            if (box.maxWidth >= 190) {
                              return Row(
                                children: [
                                  SizedBox(width: 104, child: window),
                                  const SizedBox(width: 8),
                                  Expanded(child: facts),
                                ],
                              );
                            }
                            return Column(
                              children: [
                                Flexible(child: window),
                                const SizedBox(height: 8),
                                SizedBox(height: 62, child: facts),
                              ],
                            );
                          },
                        ),
                      ),
                    ],
                  ),
                ),
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

/// 当前电平窗：数值 + 电平条 + 缓冲水位，占参考图 LCD 小窗的位置。
class _LevelWindow extends StatelessWidget {
  const _LevelWindow({
    required this.palette,
    required this.db,
    required this.accent,
    required this.bufferMs,
    required this.bufferMaxMs,
  });

  final MetalPalette palette;
  final double db;
  final Color accent;
  final double bufferMs;
  final double bufferMaxMs;

  @override
  Widget build(BuildContext context) {
    final norm = ((db + 60) / 60).clamp(0.0, 1.0);
    final barColor = norm > .94
        ? palette.salmon
        : norm > .8
        ? palette.orange
        : accent;
    final fillRatio = bufferMaxMs > 0 ? bufferMs / bufferMaxMs : 0.0;
    final bufferColor = fillRatio > .9
        ? palette.salmon
        : fillRatio > .7
        ? palette.orange
        : palette.ink;
    return NeuWell(
      radius: 10,
      palette: palette,
      deep: true,
      padding: const EdgeInsets.fromLTRB(8, 8, 8, 9),
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Text(
            '当前电平',
            style: NeuType.chrome(9.5, palette.inkFaint, tracking: .4),
          ),
          const Spacer(),
          FittedBox(
            fit: BoxFit.scaleDown,
            alignment: Alignment.centerLeft,
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.baseline,
              textBaseline: TextBaseline.alphabetic,
              children: [
                Text(
                  db.toStringAsFixed(1),
                  style: NeuType.numeric(22, barColor).copyWith(
                    shadows: [
                      Shadow(
                        color: barColor.withValues(alpha: .45),
                        blurRadius: 8,
                      ),
                    ],
                  ),
                ),
                Text(
                  ' dB',
                  style: NeuType.chrome(
                    10,
                    barColor.withValues(alpha: .8),
                    tracking: .6,
                  ),
                ),
              ],
            ),
          ),
          const SizedBox(height: 7),
          _LevelBar(norm: norm, barColor: barColor, palette: palette),
          const SizedBox(height: 5),
          // 缓冲水位：当前 / 溢出修剪上限。快到上限说明抖动正在被剪掉。
          FittedBox(
            fit: BoxFit.scaleDown,
            alignment: Alignment.centerLeft,
            child: Row(
              children: [
                Text(
                  '缓冲',
                  style: NeuType.chrome(9, palette.inkFaint, tracking: .4),
                ),
                const SizedBox(width: 4),
                Text(
                  bufferMaxMs > 0 ? bufferMs.toStringAsFixed(0) : '—',
                  style: NeuType.numeric(11, bufferColor),
                ),
                Text(
                  bufferMaxMs > 0
                      ? ' / ${bufferMaxMs.toStringAsFixed(0)} ms'
                      : '',
                  style: NeuType.chrome(
                    9,
                    palette.inkFaint,
                    tracking: .3,
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

/// 电平条：5 px 高，跟随当前 dB 归一化宽度。
class _LevelBar extends StatelessWidget {
  const _LevelBar({
    required this.norm,
    required this.barColor,
    required this.palette,
  });

  final double norm;
  final Color barColor;
  final MetalPalette palette;

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: 5,
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
                  gradient: LinearGradient(
                    colors: [barColor.withValues(alpha: .5), barColor],
                  ),
                  borderRadius: BorderRadius.circular(3),
                ),
              ),
            ),
          ],
        ),
      ),
    );
  }
}

/// 路径 / 丢帧 / 欠载，竖排三行。
class _Facts extends StatelessWidget {
  const _Facts({required this.palette, required this.channel});

  final MetalPalette palette;
  final ModuleChannel channel;

  @override
  Widget build(BuildContext context) {
    return NeuWell(
      radius: 10,
      palette: palette,
      tint: palette.wellTint,
      padding: const EdgeInsets.symmetric(horizontal: 8, vertical: 9),
      child: Column(
        mainAxisAlignment: MainAxisAlignment.spaceBetween,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _row('路径', channel.pathText),
          _row('丢帧', '${channel.lostFrames}'),
          _row('欠载', '${channel.underruns}'),
        ],
      ),
    );
  }

  Widget _row(String label, String value) {
    return Row(
      children: [
        Text(label, style: NeuType.chrome(9.5, palette.inkFaint, tracking: .4)),
        Expanded(
          child: Text(
            value,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            textAlign: TextAlign.right,
            style: NeuType.numeric(12, palette.ink),
          ),
        ),
      ],
    );
  }
}

/// 一键静音：圆形金属键 + 扬声器图标，静音时整键变红。
class _MuteButton extends StatelessWidget {
  const _MuteButton({
    required this.palette,
    required this.muted,
    required this.onTap,
  });

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
                    ? [
                        p.salmon.withValues(alpha: .95),
                        p.salmon.withValues(alpha: .7),
                      ]
                    : [p.moduleTop, p.moduleBottom],
              ),
              boxShadow: [
                BoxShadow(
                  color: muted
                      ? p.salmon.withValues(alpha: .35)
                      : p.bevel.withValues(alpha: .22),
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
