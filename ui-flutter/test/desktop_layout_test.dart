import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:siren_ui/ui_kit/device.dart';
import 'package:siren_ui/ui_kit/mobile_device.dart';
import 'package:siren_ui/ui_kit/model.dart';
import 'package:siren_ui/ui_kit/theme.dart';

/// The desktop chassis is a fixed design canvas, so a RenderFlex overflow is a
/// regression that unit tests must catch before a release build.
void main() {
  testWidgets('desktop panel fits two stream pages and inline settings', (tester) async {
    tester.view.physicalSize = const Size(1180, 760);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    final model = ShaperModel()
      ..setEqCurves([
        for (var i = 0; i < 3; i++)
          EqCurve(
            name: ['SIREN-A · 混音', 'XIAOMI-12', 'DEBIAN-HUB'][i],
            color: const [Color(0xFF5BA7FF), Color(0xFFFFA23E), Color(0xFF9B7CFF)][i],
            bands: List<double>.generate(32, (b) => .25 + (b % 7) * .08),
            gainDb: i == 0 ? 0 : -6,
            active: true,
          ),
      ])
      ..setStreamPages(2)
      ..setJoined(true)
      ..setStatusText('组网中 · 3 台')
      ..setInlineSettings(
        name: 'SIREN-PC',
        source: 'loopback',
        tone: 440,
        out: true,
        target: 120,
        master: -3,
        isBusy: false,
        role: '中心',
        link: '已连 · 2 路',
        peers: 3,
        devices: 3,
      );
    model.channels[0].setFacts(
      rms: -18.5,
      path: 'Wi-Fi',
      lost: 3,
      under: 1,
      health: 0,
      bufferMs: 128,
      bufferMaxMs: 250,
      latencyMs: 5.4,
      latencyP99Ms: 10.2,
      lossPct: 0,
    );
    model.channels[1].setFacts(
      rms: -9.2,
      path: '蓝牙',
      lost: 0,
      under: 0,
      health: 1,
      bufferMs: 232,
      bufferMaxMs: 810,
      latencyMs: 88.0,
      latencyP99Ms: 140.0,
      lossPct: 2.5,
    );

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(
          body: PulsarDevice(model: model, size: PulsarDevice.design),
        ),
      ),
    );
    await tester.pump(const Duration(milliseconds: 40));
    expect(tester.takeException(), isNull);
    expect(find.textContaining('1/2'), findsOneWidget);
    expect(find.text('当前电平'), findsNWidgets(2));
    expect(find.text('路径'), findsNWidgets(2));
    expect(find.text('丢帧'), findsNWidgets(2));
    expect(find.text('欠载'), findsNWidgets(2));
    expect(find.text('延迟 ms'), findsNWidgets(2));
    expect(find.text('5/10'), findsOneWidget);
    expect(find.text('88/140 ·3%'), findsOneWidget);
    expect(find.text('-18.5'), findsOneWidget);
    expect(find.text('Wi-Fi'), findsOneWidget);
    expect(find.text('缓冲'), findsAtLeastNWidgets(2));
    expect(find.text('128'), findsOneWidget);
    expect(find.text(' / 250 ms'), findsOneWidget);

    model.setStreamPage(1);
    await tester.pump(const Duration(milliseconds: 40));
    expect(tester.takeException(), isNull);
    expect(find.textContaining('2/2'), findsOneWidget);

    model.setTheme(model.theme == AppThemeMode.dark ? AppThemeMode.light : AppThemeMode.dark);
    await tester.pump(const Duration(milliseconds: 40));
    expect(tester.takeException(), isNull);
  });

  testWidgets('mobile chassis stacks the same cards without overflow', (tester) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);

    final model = ShaperModel()
      ..setEqCurves([
        EqCurve(
          name: 'SIREN-A',
          color: const Color(0xFF5BA7FF),
          bands: List<double>.generate(32, (b) => .2 + (b % 5) * .1),
          gainDb: 0,
          active: true,
        ),
      ])
      ..setJoined(true)
      ..setStatusText('组网中 · 2 台');
    model.channels[0].setFacts(
      rms: -15.5,
      path: 'Wi-Fi',
      lost: 0,
      under: 0,
      health: 0,
      bufferMs: 96,
      bufferMaxMs: 250,
      latencyMs: 12.0,
      latencyP99Ms: 34.0,
      lossPct: 0,
    );

    await tester.pumpWidget(
      MaterialApp(
        home: Scaffold(body: MobileDevice(model: model, size: MobileDevice.design)),
      ),
    );
    await tester.pump(const Duration(milliseconds: 40));
    expect(tester.takeException(), isNull);
    expect(find.text('本机'), findsOneWidget);
    expect(find.text('组网'), findsOneWidget);
    expect(find.text('12/34'), findsOneWidget);
    expect(find.text('当前电平'), findsNWidgets(2));
    expect(find.text('缓冲'), findsWidgets);
    expect(find.text('BYPASS'), findsOneWidget);
  });
}
