import 'dart:io' show Platform;
import 'dart:ui' show AppExitResponse;

import 'package:flutter/material.dart';
import 'package:window_manager/window_manager.dart';

import 'app_model.dart';
import 'rust_api.dart';
import 'theme.dart';
import 'tantrum/model.dart';
import 'tantrum/tantrum_page.dart';
import 'ui_kit/siren_page.dart';

/// `--ui=tantrum` 或 SIREN_UI=tantrum 打开新控制台（纯 UI，不接引擎）。
bool _tantrumRequested(List<String> args) =>
    args.any((a) => a == '--ui=tantrum') ||
    Platform.environment['SIREN_UI'] == 'tantrum';

Future<void> main(List<String> args) async {
  WidgetsFlutterBinding.ensureInitialized();
  final tantrum = _tantrumRequested(args);
  if (!tantrum) {
    try {
      rust = RustApi.load();
      rust.initLogging();
    } catch (error) {
      runApp(_BootstrapError(message: '$error'));
      return;
    }
  }
  if (Platform.isWindows || Platform.isLinux || Platform.isMacOS) {
    await windowManager.ensureInitialized();
    const options = WindowOptions(
      size: Size(1240, 800),
      minimumSize: Size(980, 640),
      center: true,
      backgroundColor: Color(0xFF10151A),
      skipTaskbar: false,
      title: 'SirenStudio',
    );
    await windowManager.waitUntilReadyToShow(options, () async {
      await windowManager.setAsFrameless();
      await windowManager.setResizable(true);
      await windowManager.show();
      await windowManager.focus();
    });
  }
  runApp(SirenApp(tantrum: tantrum));
}

class SirenApp extends StatefulWidget {
  const SirenApp({super.key, this.tantrum = false});

  /// true 时只渲染新控制台，不创建 AppModel、不碰 FFI
  final bool tantrum;

  @override
  State<SirenApp> createState() => _SirenAppState();
}

class _SirenAppState extends State<SirenApp> {
  late final AppModel? _model;
  final TantrumModel _tantrumModel = TantrumModel();
  AppLifecycleListener? _lifecycle;

  @override
  void initState() {
    super.initState();
    if (widget.tantrum) {
      return;
    }
    _model = AppModel();
    _lifecycle = AppLifecycleListener(
      onExitRequested: () async {
        await _model?.shutdown();
        return AppExitResponse.exit;
      },
    );
  }

  @override
  void dispose() {
    _lifecycle?.dispose();
    _model?.dispose();
    super.dispose();
  }

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      title: 'SirenStudio',
      debugShowCheckedModeBanner: false,
      theme: buildAppTheme(),
      home: widget.tantrum
          ? TantrumPage(model: _tantrumModel)
          : SirenPage(model: _model!),
    );
  }
}

class _BootstrapError extends StatelessWidget {
  const _BootstrapError({required this.message});

  final String message;

  @override
  Widget build(BuildContext context) {
    return MaterialApp(
      debugShowCheckedModeBanner: false,
      theme: buildAppTheme(),
      home: Scaffold(
        body: Center(
          child: ConstrainedBox(
            constraints: const BoxConstraints(maxWidth: 560),
            child: Padding(
              padding: const EdgeInsets.all(32),
              child: Column(
                mainAxisSize: MainAxisSize.min,
                children: [
                  const Icon(
                    Icons.error_outline_rounded,
                    size: 42,
                    color: AppColors.rose,
                  ),
                  const SizedBox(height: 16),
                  const Text(
                    'Rust 核心初始化失败',
                    style: TextStyle(
                      fontSize: 18,
                      fontWeight: FontWeight.w700,
                    ),
                  ),
                  const SizedBox(height: 10),
                  Text(
                    message,
                    textAlign: TextAlign.center,
                    style: const TextStyle(
                      color: AppColors.textDim,
                      fontSize: 12,
                      height: 1.5,
                    ),
                  ),
                  const SizedBox(height: 18),
                  const Text(
                    '请先用 scripts/build_ui.ps1 构建，确保 siren_ui.dll 与 exe 同目录。',
                    textAlign: TextAlign.center,
                    style: TextStyle(
                      color: AppColors.textFaint,
                      fontSize: 11,
                    ),
                  ),
                ],
              ),
            ),
          ),
        ),
      ),
    );
  }
}
