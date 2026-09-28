import { useEffect, useState } from 'react';

import { Avatar, Button, Card, Chip, Input, NumberField, Radio, RadioGroup, Label, Separator, toast } from '@heroui/react';
import { ExternalLink, FolderOpen, LogOut, RotateCw } from 'lucide-react';
import { useShallow } from 'zustand/react/shallow';

import type { AppInfo, Preferences, ProxySettings, Track } from '@shared/protocol';
import { TRACKS } from '@shared/protocol';

import { errorMessage, isMac, shell } from '../api';
import { CheckBox, PageHeader, SettingRow, Toggle } from '../components/common';
import { TRACK_LABELS } from '../format';
import { useStore } from '../store';

const REPOSITORY = 'https://github.com/Uniseem/SJTU-CANVAS-DOWNLOADER';

export function SettingsPage() {
  const { settings, account, engineState, save, logout, restart } = useStore(
    useShallow((state) => ({
      settings: state.settings,
      account: state.account,
      engineState: state.engine,
      save: state.savePreferences,
      logout: state.logout,
      restart: state.restartEngine,
    })),
  );
  const [info, setInfo] = useState<AppInfo | null>(null);
  const [busy, setBusy] = useState(false);
  const [proxyUrl, setProxyUrl] = useState('');

  useEffect(() => {
    void shell.appInfo().then(setInfo);
  }, []);

  const preferences = settings?.preferences;
  useEffect(() => {
    setProxyUrl(preferences?.proxy.mode === 'custom' ? preferences.proxy.url : '');
  }, [preferences]);

  if (!settings || !preferences) {
    return <PageHeader title="设置" />;
  }

  const update = async (patch: Partial<Preferences>): Promise<void> => {
    setBusy(true);
    try {
      await save({ ...preferences, ...patch });
    } catch (error) {
      toast.danger('无法保存设置', { description: errorMessage(error) });
    } finally {
      setBusy(false);
    }
  };

  const setProxy = (proxy: ProxySettings): Promise<void> => update({ proxy });
  const isDefaultDir = preferences.download_dir === settings.default_download_dir;
  const profile = account?.profile;

  return (
    <>
      <PageHeader title="设置" />
      <div className="min-h-0 flex-1 overflow-x-hidden overflow-y-auto px-8 pb-10">
        <div className="mx-auto flex max-w-3xl flex-col gap-5">
          <Card>
            <Card.Header>
              <Card.Title>下载</Card.Title>
              <Card.Description>
                课堂录像保存在“课程名 [课程号]/课堂录像/日期 讲次/”下，课程文件保存在“课程文件”文件夹；已有的文件不会被覆盖。
              </Card.Description>
            </Card.Header>
            <Card.Content className="divide-y divide-border">
              <SettingRow title="保存位置" description={<span className="selectable break-all">{preferences.download_dir}</span>}>
                <Button
                  size="sm"
                  variant="secondary"
                  isIconOnly
                  aria-label={isMac ? '在访达中显示' : '打开文件夹'}
                  onPress={() => void shell.openPath(preferences.download_dir)}
                >
                  <FolderOpen size={15} />
                </Button>
                {!isDefaultDir ? (
                  <Button size="sm" variant="secondary" isDisabled={busy} onPress={() => void update({ download_dir: settings.default_download_dir })}>
                    恢复默认
                  </Button>
                ) : null}
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={busy}
                  onPress={async () => {
                    const chosen = await shell.chooseFolder(preferences.download_dir);
                    if (chosen) {
                      await update({ download_dir: chosen });
                    }
                  }}
                >
                  更改…
                </Button>
              </SettingRow>
              <SettingRow title="每次下载前选择保存位置" description="打开时，点“下载”会先询问保存到哪个文件夹。">
                <Toggle
                  ariaLabel="每次下载前选择保存位置"
                  isSelected={preferences.ask_destination}
                  isDisabled={busy}
                  onChange={(value) => void update({ ask_destination: value })}
                />
              </SettingRow>
              <SettingRow
                title="同时下载的文件数"
                description={`同时下载更多文件更能跑满带宽；学校服务繁忙或网络不稳定时可以调低。最多 ${settings.concurrency_max} 个。`}
              >
                <NumberField
                  aria-label="同时下载的文件数"
                  value={preferences.concurrency}
                  minValue={1}
                  maxValue={settings.concurrency_max}
                  step={1}
                  isDisabled={busy}
                  onChange={(value) => {
                    if (Number.isFinite(value) && value !== preferences.concurrency) {
                      void update({ concurrency: value });
                    }
                  }}
                  className="w-36"
                >
                  <NumberField.Group>
                    <NumberField.DecrementButton />
                    <NumberField.Input />
                    <NumberField.IncrementButton />
                  </NumberField.Group>
                </NumberField>
              </SettingRow>
              <SettingRow title="默认下载的画面" description="打开课程时预先选中的画面，可以在每门课里临时更改。合成画面并非每节课都有。">
                <div className="flex flex-col gap-2">
                  {TRACKS.map((track: Track) => {
                    const selected = preferences.default_tracks.includes(track);
                    return (
                      <CheckBox
                        key={track}
                        label={TRACK_LABELS[track]}
                        isSelected={selected}
                        isDisabled={busy || (selected && preferences.default_tracks.length === 1)}
                        onChange={(value) =>
                          void update({
                            default_tracks: TRACKS.filter((item) => (item === track ? value : preferences.default_tracks.includes(item))),
                          })
                        }
                      />
                    );
                  })}
                </div>
              </SettingRow>
            </Card.Content>
          </Card>

          <Card>
            <Card.Header>
              <Card.Title>网络</Card.Title>
              <Card.Description>
                学校服务一般直接连接即可；系统代理（如全局 VPN）导致无法访问时，可以选择“不使用代理”。自定义代理支持 http://、https:// 和
                socks5:// 地址。
              </Card.Description>
            </Card.Header>
            <Card.Content>
              <RadioGroup
                aria-label="代理"
                value={preferences.proxy.mode}
                isDisabled={busy}
                onChange={(mode) => {
                  if (mode === 'system' || mode === 'direct') {
                    void setProxy({ mode });
                  } else if (mode === 'custom' && proxyUrl.trim()) {
                    void setProxy({ mode: 'custom', url: proxyUrl.trim() });
                  } else if (mode === 'custom') {
                    // Applied once an address is entered.
                    toast.info('请输入代理地址后点“应用”');
                  }
                }}
              >
                <ProxyOption value="system" label="跟随系统设置" />
                <ProxyOption value="direct" label="不使用代理" />
                <ProxyOption value="custom" label="自定义代理" />
              </RadioGroup>
              <div className="mt-3 flex items-center gap-2">
                <Input
                  aria-label="代理地址"
                  placeholder="http://127.0.0.1:7890"
                  value={proxyUrl}
                  onChange={(event) => setProxyUrl(event.target.value)}
                  variant="secondary"
                  className="max-w-sm"
                />
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={busy || !proxyUrl.trim() || (preferences.proxy.mode === 'custom' && preferences.proxy.url === proxyUrl.trim())}
                  onPress={() => void setProxy({ mode: 'custom', url: proxyUrl.trim() })}
                >
                  应用
                </Button>
              </div>
            </Card.Content>
          </Card>

          <Card>
            <Card.Header>
              <Card.Title>Canvas 账户</Card.Title>
            </Card.Header>
            <Card.Content>
              {profile ? (
                <div className="flex items-center gap-4">
                  <Avatar size="lg">
                    {profile.avatar_url ? <Avatar.Image src={profile.avatar_url} alt="" /> : null}
                    <Avatar.Fallback>{profile.name.slice(0, 1)}</Avatar.Fallback>
                  </Avatar>
                  <div className="min-w-0 flex-1">
                    <div className="truncate font-medium">{profile.name}</div>
                    <div className="mt-1 flex flex-wrap items-center gap-2 text-xs text-muted">
                      {account?.persisted ? (
                        <span>登录状态已加密保存，下次打开无需再扫码</span>
                      ) : (
                        <Chip size="sm" color="warning" variant="soft">
                          无法使用系统密钥保护登录，退出应用后需要重新扫码
                        </Chip>
                      )}
                    </div>
                  </div>
                  <Button
                    variant="danger-soft"
                    size="sm"
                    isDisabled={busy}
                    onPress={async () => {
                      setBusy(true);
                      try {
                        await logout();
                      } catch (error) {
                        toast.danger('无法退出登录', { description: errorMessage(error) });
                      } finally {
                        setBusy(false);
                      }
                    }}
                  >
                    <LogOut size={14} />
                    退出登录
                  </Button>
                </div>
              ) : (
                <p className="text-sm text-muted">未登录</p>
              )}
            </Card.Content>
          </Card>

          <Card>
            <Card.Header>
              <Card.Title>关于</Card.Title>
              <Card.Description>SJTU Canvas Downloader，以 MIT 许可证发布。</Card.Description>
            </Card.Header>
            <Card.Content className="divide-y divide-border">
              <SettingRow title="版本" description={`应用 ${info?.version ?? '…'} · 引擎 ${engineState?.version ?? settings.engine_version}`}>
                <Button size="sm" variant="secondary" onPress={() => void shell.openExternal(`${REPOSITORY}/releases`)}>
                  <ExternalLink size={14} />
                  检查更新
                </Button>
              </SettingRow>
              <SettingRow title="数据目录" description={<span className="selectable break-all">{settings.data_dir}</span>}>
                <Button size="sm" variant="secondary" onPress={() => void shell.openPath(settings.data_dir)}>
                  <FolderOpen size={14} />
                  打开
                </Button>
                <Button size="sm" variant="secondary" onPress={() => info && void shell.openPath(info.logsDir)}>
                  日志
                </Button>
              </SettingRow>
              <SettingRow title="下载引擎" description="遇到问题时可以重新启动引擎，正在进行的下载会自动继续。">
                <Button
                  size="sm"
                  variant="secondary"
                  isDisabled={busy}
                  onPress={async () => {
                    setBusy(true);
                    try {
                      await restart();
                    } finally {
                      setBusy(false);
                    }
                  }}
                >
                  <RotateCw size={14} />
                  重新启动引擎
                </Button>
              </SettingRow>
              {settings.test_mode ? (
                <SettingRow title="测试模式">
                  <Chip size="sm" color="warning" variant="soft">
                    {settings.fake_school ? '演示学校' : '测试模式'}
                  </Chip>
                </SettingRow>
              ) : null}
            </Card.Content>
            <Card.Footer>
              <Separator className="mb-3" />
              <p className="text-xs leading-relaxed text-muted">
                项目主页：
                <button type="button" className="text-link hover:underline" onClick={() => void shell.openExternal(REPOSITORY)}>
                  {REPOSITORY.replace('https://', '')}
                </button>
              </p>
            </Card.Footer>
          </Card>
        </div>
      </div>
    </>
  );
}

function ProxyOption({ value, label }: { value: string; label: string }) {
  return (
    <Radio value={value}>
      <Radio.Content>
        <Radio.Control>
          <Radio.Indicator />
        </Radio.Control>
        <Label>{label}</Label>
      </Radio.Content>
    </Radio>
  );
}
