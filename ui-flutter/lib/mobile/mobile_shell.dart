import 'package:flutter/material.dart';

import '../app_model.dart';
import '../theme.dart';
import '../widgets.dart';
import 'mesh_mobile.dart';

/// Mobile shell: a single mesh page. There is no center/sender split — every
/// node receives and sends, so there is nothing to configure beyond the local
/// capture source.
class MobileShell extends StatefulWidget {
  const MobileShell({super.key, required this.model});

  final AppModel model;

  @override
  State<MobileShell> createState() => _MobileShellState();
}

class _MobileShellState extends State<MobileShell> {
  @override
  Widget build(BuildContext context) {
    final model = widget.model;
    return Scaffold(
      backgroundColor: AppColors.bg,
      body: SafeArea(
        bottom: false,
        child: Column(
          children: [
            _Header(model: model),
            Expanded(child: MeshMobile(model: model)),
          ],
        ),
      ),
    );
  }
}

class _Header extends StatelessWidget {
  const _Header({required this.model});

  final AppModel model;

  @override
  Widget build(BuildContext context) {
    return Padding(
      padding: const EdgeInsets.fromLTRB(18, 10, 18, 10),
      child: AnimatedBuilder(
        animation: model,
        builder: (context, _) {
          final label = model.meshRunning
              ? '组网中'
              : model.busyMesh
                  ? '加入中'
                  : '未组网';
          final color = model.meshRunning
              ? AppColors.green
              : model.busyMesh
                  ? AppColors.amber
                  : AppColors.textFaint;
          return Row(
            children: [
              Container(
                width: 34,
                height: 34,
                decoration: BoxDecoration(
                  gradient: const LinearGradient(
                    colors: [AppColors.accent, AppColors.accent2],
                  ),
                  borderRadius: BorderRadius.circular(10),
                ),
                child: const Icon(
                  Icons.graphic_eq_rounded,
                  color: AppColors.ink,
                  size: 20,
                ),
              ),
              const SizedBox(width: 10),
              const Column(
                crossAxisAlignment: CrossAxisAlignment.start,
                children: [
                  Text(
                    'SIREN',
                    style: TextStyle(
                      color: AppColors.text,
                      fontSize: 15,
                      fontWeight: FontWeight.w800,
                      letterSpacing: 3,
                    ),
                  ),
                  Text(
                    'STUDIO',
                    style: TextStyle(
                      color: AppColors.textFaint,
                      fontSize: 8,
                      letterSpacing: 4,
                    ),
                  ),
                ],
              ),
              const Spacer(),
              StatusPill(label: label, color: color),
              const SizedBox(width: 8),
              IconButton(
                tooltip: '运行日志',
                onPressed: () => _showLogs(context),
                icon: const Icon(Icons.terminal_rounded,
                    color: AppColors.textDim, size: 20),
              ),
            ],
          );
        },
      ),
    );
  }

  void _showLogs(BuildContext context) {
    showModalBottomSheet<void>(
      context: context,
      backgroundColor: AppColors.bg2,
      isScrollControlled: true,
      shape: const RoundedRectangleBorder(
        borderRadius: BorderRadius.vertical(top: Radius.circular(22)),
      ),
      builder: (context) {
        return AnimatedBuilder(
          animation: model,
          builder: (context, _) {
            final logs = model.logs.reversed.toList(growable: false);
            return SafeArea(
              child: Container(
                constraints: BoxConstraints(
                  maxHeight: MediaQuery.of(context).size.height * 0.6,
                ),
                padding: const EdgeInsets.fromLTRB(18, 14, 18, 18),
                child: Column(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  mainAxisSize: MainAxisSize.min,
                  children: [
                    const Text(
                      '运行日志',
                      style: TextStyle(
                        color: AppColors.text,
                        fontSize: 14,
                        fontWeight: FontWeight.w700,
                      ),
                    ),
                    const SizedBox(height: 10),
                    Flexible(
                      child: logs.isEmpty
                          ? const Padding(
                              padding: EdgeInsets.all(16),
                              child: Text(
                                '暂无日志',
                                style: TextStyle(
                                    color: AppColors.textFaint, fontSize: 12),
                              ),
                            )
                          : ListView.separated(
                              shrinkWrap: true,
                              itemCount: logs.length,
                              separatorBuilder: (_, _) =>
                                  const SizedBox(height: 6),
                              itemBuilder: (context, index) {
                                final entry = logs[index];
                                final time = entry.at.toIso8601String()
                                    .substring(11, 19);
                                return Row(
                                  crossAxisAlignment: CrossAxisAlignment.start,
                                  children: [
                                    Text(
                                      time,
                                      style: AppText.mono.copyWith(
                                        color: AppColors.textFaint,
                                        fontSize: 10.5,
                                      ),
                                    ),
                                    const SizedBox(width: 10),
                                    Expanded(
                                      child: Text(
                                        entry.message,
                                        style: AppText.mono.copyWith(
                                          color: entry.isError
                                              ? AppColors.rose
                                              : AppColors.textDim,
                                          fontSize: 11,
                                          height: 1.35,
                                        ),
                                      ),
                                    ),
                                  ],
                                );
                              },
                            ),
                    ),
                  ],
                ),
              ),
            );
          },
        );
      },
    );
  }
}
