import { useEffect, useRef, useState } from 'react';
import { CalendarClock, ScrollText, Trash2 } from 'lucide-react';
import { useTranslations } from 'use-intl';
import { useClearLogs } from '@/api/log';
import { useSettingList, useSetSetting, SettingKey } from '@/api/setting';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { toast } from 'sonner';

// SettingLog 提供日志保留天数设置与完成日志清空操作。
export function SettingLog() {
    const t = useTranslations('setting');
    const clearLogs = useClearLogs();
    const { data: settings } = useSettingList();
    const setSetting = useSetSetting();

    const [retentionDays, setRetentionDays] = useState('');
    const initialRetention = useRef('');

    useEffect(() => {
        if (!settings) return;
        const retention = settings.find((s) => s.key === SettingKey.LogRetentionDays);
        if (retention) {
            queueMicrotask(() => setRetentionDays(retention.value));
            initialRetention.current = retention.value;
        }
    }, [settings]);

    const handleSaveRetention = () => {
        const value = retentionDays.trim();
        if (value === initialRetention.current) return;
        const parsed = Number(value);
        if (Number.isNaN(parsed) || parsed < 1) {
            toast.error(t('log.retention.invalid'));
            setRetentionDays(initialRetention.current);
            return;
        }
        setSetting.mutate({ key: SettingKey.LogRetentionDays, value: String(Math.floor(parsed)) }, {
            onSuccess: () => {
                toast.success(t('saved'));
                initialRetention.current = String(Math.floor(parsed));
            },
            onError: () => {
                setRetentionDays(initialRetention.current);
                toast.error(t('log.retention.saveFailed'));
            },
        });
    };

    const handleClearLogs = () => {
        clearLogs.mutate(undefined, {
            onSuccess: () => toast.success(t('log.clearSuccess')),
            onError: () => toast.error(t('log.clearFailed')),
        });
    };

    return (
        <div className="space-y-5 rounded-3xl border border-border bg-card p-6">
            <h2 className="flex items-center gap-2 text-lg font-bold text-card-foreground">
                <ScrollText className="size-5" />
                {t('log.title')}
            </h2>

            {/* 日志保留天数 */}
            <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <CalendarClock className="size-5 text-muted-foreground" />
                    <div className="flex flex-col">
                        <span className="text-sm font-medium">{t('log.retention.label')}</span>
                        <span className="text-xs text-muted-foreground">{t('log.retention.hint')}</span>
                    </div>
                </div>
                <Input
                    type="number"
                    min={1}
                    value={retentionDays}
                    onChange={(e) => setRetentionDays(e.target.value)}
                    onBlur={handleSaveRetention}
                    placeholder={t('log.retention.placeholder')}
                    className="w-32 rounded-xl"
                />
            </div>

            {/* 清空日志 */}
            <div className="flex items-center justify-between gap-4">
                <div className="flex items-center gap-3">
                    <Trash2 className="size-5 text-muted-foreground" />
                    <span className="text-sm font-medium">{t('log.clear.label')}</span>
                </div>
                <Button
                    variant="destructive"
                    size="sm"
                    onClick={handleClearLogs}
                    disabled={clearLogs.isPending}
                    className="rounded-xl"
                >
                    {clearLogs.isPending ? t('log.clear.clearing') : t('log.clear.button')}
                </Button>
            </div>
        </div>
    );
}
