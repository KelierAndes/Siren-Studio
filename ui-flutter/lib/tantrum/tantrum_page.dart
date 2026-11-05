import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

import 'controls.dart';
import 'model.dart';
import 'panels.dart';
import 'tokens.dart';

/// 新控制台（Windows 桌面整机）。排版按参考图 1 的四段带：
/// 标题条 → 左屏/右格栅 → 读数条 → 单行控件；控件外形用参考图 2 的 token。
/// 只有 UI：数据全部来自 [TantrumModel] 的占位值，未接引擎。
class TantrumPage extends StatefulWidget {
  const TantrumPage({super.key, required this.model});

  final TantrumModel model;

  @override
  State<TantrumPage> createState() => _TantrumPageState();
}

class _TantrumPageState extends State<TantrumPage> {
  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: widget.model,
      builder: (context, _) => LayoutBuilder(
        builder: (context, box) {
          final widest = math.min(box.maxWidth - 48, 1560.0);
          final tallest = (box.maxHeight - 56) * TantrumTokens.panelRatio;
          final width = (widest < tallest ? widest : tallest).clamp(380.0, 1560.0);
          return _Desk(
            child: TantrumShell(
              width: width,
              child: _Panel(model: widget.model, width: width),
            ),
          );
        },
      ),
    );
  }
}

/// 桌面层：可拖动窗口 + 右上角窗口按钮（无边框窗口需要的最小窗控）
class _Desk extends StatelessWidget {
  const _Desk({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      behavior: HitTestBehavior.translucent,
      onPanStart: (_) => windowManager.startDragging(),
      child: Stack(
        fit: StackFit.expand,
        children: [
          const DecoratedBox(
            decoration: BoxDecoration(
              // 桌面：左上打光、四周压暗，机箱的投影才落得住
              gradient: RadialGradient(
                center: Alignment(-0.55, -0.75),
                radius: 1.35,
                colors: [
                  TantrumTokens.deskBottom,
                  TantrumTokens.deskTop,
                  Color(0xFFB9C2CD),
                ],
                stops: [0.0, 0.55, 1.0],
              ),
            ),
          ),
          child,
          Positioned(
            right: 10,
            top: 8,
            child: Row(
              children: [
                const _DeskButton(icon: Icons.remove_rounded, action: _minimize),
                const SizedBox(width: 6),
                const _DeskButton(icon: Icons.crop_square_rounded, action: _toggleMaximize),
                const SizedBox(width: 6),
                const _DeskButton(icon: Icons.close_rounded, action: _close),
              ],
            ),
          ),
        ],
      ),
    );
  }
}

Future<void> _minimize() => windowManager.minimize();

Future<void> _toggleMaximize() async {
  if (await windowManager.isMaximized()) {
    await windowManager.unmaximize();
  } else {
    await windowManager.maximize();
  }
}

Future<void> _close() => windowManager.close();

class _DeskButton extends StatelessWidget {
  const _DeskButton({required this.icon, required this.action});

  final IconData icon;
  final Future<void> Function() action;

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      onTap: () => action(),
      child: Container(
        width: 26,
        height: 22,
        alignment: Alignment.center,
        decoration: BoxDecoration(
          color: Colors.white.withValues(alpha: 0.55),
          borderRadius: BorderRadius.circular(6),
        ),
        child: Icon(icon, size: 13, color: TantrumTokens.ink),
      ),
    );
  }
}

/// 面板内部：四段带的绝对定位。所有坐标由 [TantrumMetrics] 从面板宽度推导，
/// 因此窗口任意缩放都不会出现控件互相重叠。
class _Panel extends StatelessWidget {
  const _Panel({required this.model, required this.width});

  final TantrumModel model;
  final double width;

  @override
  Widget build(BuildContext context) {
    final m = TantrumMetrics(width);
    return Stack(
      children: [
        Positioned(
          left: m.inset,
          right: m.inset,
          top: m.headerTop,
          child: TantrumHeader(
            m: m,
            brand: 'SIREN',
            preset: '0${model.presetIndex + 1} ${model.preset}',
            onPrev: () => model.stepPreset(-1),
            onNext: () => model.stepPreset(1),
            powerOn: model.powerOn,
            onPower: () => model.setPower(!model.powerOn),
          ),
        ),
        Positioned(
          left: m.screenLeftX,
          top: m.bodyTop,
          width: m.screenWidth,
          height: m.bodyHeight,
          child: model.screenMode == 0
              ? TantrumScope(
                  m: m,
                  waveform: model.waveform,
                  title: '主输出',
                  subtitle: '${model.outText} dBFS',
                )
              : TantrumEqGraph(m: m, curve: model.eqCurve, spectrum: model.spectrum),
        ),
        Positioned(
          left: m.grilleLeftX,
          top: m.bodyTop,
          width: m.grilleWidth,
          height: m.bodyHeight,
          child: model.advanced
              ? _FaderBank(m: m, model: model)
              : TantrumGrille(m: m, centerLabel: 'OUT'),
        ),
        // 图1 两侧的 IN / OUT 电平柱
        Positioned(
          left: m.meterLeftX,
          top: m.bodyTop,
          width: m.meterWidth,
          height: m.bodyHeight,
          child: TantrumLevelMeter(
              m: m, label: 'IN', level: model.inLevel, text: model.inText),
        ),
        Positioned(
          left: m.meterRightX,
          top: m.bodyTop,
          width: m.meterWidth,
          height: m.bodyHeight,
          child: TantrumLevelMeter(
              m: m, label: 'OUT', level: model.outLevel, text: model.outText),
        ),
        // 读数条左半：屏模式胶囊 + ADV
        Positioned(
          left: m.meterLeftX,
          top: m.readoutTop,
          width: m.meterWidth + m.paneGap + m.screenWidth,
          height: m.readoutHeight,
          child: FittedBox(
            fit: BoxFit.scaleDown,
            alignment: Alignment.centerLeft,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                TantrumSegPill(
                  items: const ['电平', '均衡'],
                  index: model.screenMode,
                  m: m,
                  onSelect: model.setScreenMode,
                ),
                SizedBox(width: 10 * m.scale),
                Text('ADV', style: TantrumTokens.caption(m.scale)),
                SizedBox(width: 5 * m.scale),
                TantrumSwitch(
                  on: model.advanced,
                  m: m,
                  onChanged: model.setAdvanced,
                ),
              ],
            ),
          ),
        ),
        // 读数条右半：红色数值读数（图1 的 PHASE AMOUNT 21 位置）
        Positioned(
          right: m.inset,
          top: m.readoutTop,
          width: m.grilleWidth,
          height: m.readoutHeight,
          child: FittedBox(
            fit: BoxFit.scaleDown,
            alignment: Alignment.centerRight,
            child: Row(
              mainAxisSize: MainAxisSize.min,
              children: [
                for (final stat in model.stats) ...[
                  TantrumReadout(label: stat.label, value: stat.value, m: m),
                  SizedBox(width: 12 * m.scale),
                ],
              ],
            ),
          ),
        ),
        Positioned(
          left: 0,
          right: 0,
          top: m.controlTop,
          height: m.controlHeight,
          child: _ControlRow(m: m, model: model),
        ),
      ],
    );
  }
}

/// 单行控件：5 个旋钮 + 中间的箭头/胶囊 + LED（图1 的数量与位置）
class _ControlRow extends StatelessWidget {
  const _ControlRow({required this.m, required this.model});

  final TantrumMetrics m;
  final TantrumModel model;

  @override
  Widget build(BuildContext context) {
    final centers = m.knobCenters;
    final items = <_KnobSpec>[
      _KnobSpec('缓冲目标', model.bufferTargetText, model.bufferTarget, model.setBufferTarget),
      _KnobSpec('帧长', model.frameLengthText, model.frameLength, model.setFrameLength),
      _KnobSpec('混音增益', model.mixGainText, model.mixGain, model.setMixGain),
      _KnobSpec('输出增益', model.outputGainText, model.outputGain, model.setOutputGain),
      _KnobSpec('发送增益', model.sendGainText, model.sendGain, model.setSendGain),
    ];
    final half = 62 * m.scale;
    return Stack(
      children: [
        for (var i = 0; i < items.length; i++)
          Positioned(
            left: centers[i] - half,
            top: 0,
            width: half * 2,
            height: m.controlHeight,
            child: Column(
              children: [
                Text(items[i].label,
                    style: TantrumTokens.caption(m.scale, color: TantrumTokens.ink)),
                SizedBox(height: 4 * m.scale),
                TantrumKnob(
                  key: ValueKey('knob$i'),
                  value: items[i].value,
                  m: m,
                  onChanged: items[i].onChanged,
                ),
                SizedBox(height: 2 * m.scale),
                Text(items[i].valueText, style: TantrumTokens.value(m.scale)),
              ],
            ),
          ),
        // 中间：上下箭头 + 胶囊（图1 的 TUBE 位）
        Positioned(
          left: m.centerClusterX - 34 * m.scale,
          top: 6 * m.scale,
          width: 68 * m.scale,
          child: Column(
            children: [
              TantrumChevronPair(m: m, onUp: () => model.stepPreset(1), onDown: () => model.stepPreset(-1)),
              SizedBox(height: 5 * m.scale),
              TantrumCapsule(
                label: model.mixing ? '混音' : '直通',
                raised: model.mixing,
                m: m,
                onTap: () => model.setMixing(!model.mixing),
              ),
            ],
          ),
        ),
        // LED：图1 里第 2 个旋钮右侧那颗红点
        Positioned(
          left: centers[1] + 40 * m.scale,
          top: 30 * m.scale,
          child: TantrumLed(on: true, m: m),
        ),
      ],
    );
  }
}

class _KnobSpec {
  const _KnobSpec(this.label, this.valueText, this.value, this.onChanged);
  final String label;
  final String valueText;
  final double value;
  final ValueChanged<double> onChanged;
}

/// ADV 打开后右格栅换成四推子（图2 的帽式推子）
class _FaderBank extends StatelessWidget {
  const _FaderBank({required this.m, required this.model});

  final TantrumMetrics m;
  final TantrumModel model;

  static const _labels = ['低频', '中频', '高频', '空气'];

  @override
  Widget build(BuildContext context) {
    return Container(
      decoration: BoxDecoration(
        borderRadius: BorderRadius.circular(7 * m.scale),
        gradient: LinearGradient(
          begin: const Alignment(-0.3, -1),
          end: const Alignment(0.3, 1),
          colors: [Colors.white, TantrumTokens.panelBottom],
        ),
        boxShadow: [
          BoxShadow(
            color: TantrumTokens.castDark,
            blurRadius: 5 * m.scale,
            offset: Offset(1.4 * m.scale, 2.4 * m.scale),
          ),
          BoxShadow(
            color: TantrumTokens.rimLight,
            blurRadius: 3 * m.scale,
            offset: Offset(-1 * m.scale, -1.2 * m.scale),
          ),
        ],
      ),
      padding: EdgeInsets.symmetric(vertical: 6 * m.scale),
      child: Row(
        mainAxisAlignment: MainAxisAlignment.spaceEvenly,
        children: [
          for (var i = 0; i < model.bands.length; i++)
            TantrumFader(
              value: model.bands[i],
              m: m,
              label: _labels[i],
              onChanged: (v) => model.setBand(i, v),
            ),
        ],
      ),
    );
  }
}
