import { memo, useEffect, useMemo, useState, type CSSProperties } from 'react';
import { AlertCircle, ArrowDownToLine, ArrowRight, ArrowUpFromLine, Clock, Cpu, Database, DollarSign, Loader2, Square } from 'lucide-react';
import { useTranslations } from 'use-intl';
import JsonView from '@uiw/react-json-view';
import { githubDarkTheme } from '@uiw/react-json-view/githubDark';
import { githubLightTheme } from '@uiw/react-json-view/githubLight';
import { useTheme } from '@/provider/theme';
import { type RelayLogOverview, type RoundLog, useLogDetail, useLogRequestBody, useLogResponseBody, useStopRound } from '@/api/log';
import { useGroupList, useUpdateGroupActiveItem } from '@/api/group';
import { useChannelList } from '@/api/channel';
import { getModelIcon } from '@/lib/model-icons';
import { Badge } from '@/components/ui/badge';
import { Tabs, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { cn } from '@/lib/utils';
import { CopyIconButton } from '@/components/common/CopyButton';
import { toast } from 'sonner';
import { MemberStatus } from '@/components/modules/group/MemberStatus';
import {
    MorphingDialog,
    MorphingDialogTrigger,
    MorphingDialogContainer,
    MorphingDialogContent,
    MorphingDialogClose,
    MorphingDialogTitle,
    MorphingDialogDescription,
    useMorphingDialog,
} from '@/components/ui/morphing-dialog';

// formatTime 将后端 RFC3339 时间转换为本地时分秒。
function formatTime(value: string) {
    const date = new Date(value);
    if (Number.isNaN(date.getTime()) || date.getUTCFullYear() === 1) return '--';
    const now = new Date();
    const isSameDay =
        date.getFullYear() === now.getFullYear() &&
        date.getMonth() === now.getMonth() &&
        date.getDate() === now.getDate();
    const time = date.toLocaleTimeString(undefined, {
        hour: '2-digit',
        minute: '2-digit',
        second: '2-digit',
        hour12: false,
    });
    if (isSameDay) return time;
    const month = String(date.getMonth() + 1).padStart(2, '0');
    const day = String(date.getDate()).padStart(2, '0');
    return `${month}-${day} ${time}`;
}

// formatMilliseconds 将毫秒转换为紧凑耗时文本。
function formatMilliseconds(value: number) {
    const milliseconds = Math.max(0, value);
    if (milliseconds < 1000) return `${Math.round(milliseconds)}ms`;
    return `${(milliseconds / 1000).toFixed(2)}s`;
}

// LogMetrics 渲染耗时, 费用和 Token 指标; card 变体用于卡片栅格, footer 变体用于弹窗底部。
function LogMetrics({ log, now, brandColor, variant }: { log: RelayLogOverview; now: number; brandColor: string; variant: 'card' | 'footer' }) {
    const cachedTokens = log.usage.cache_read_tokens ?? 0;
    // 进行中的请求按共享时钟推算耗时, 结束后改用后端记录的最终耗时。
    const duration = log.status === 'running' || log.status === 'committed'
        ? formatMilliseconds(now - new Date(log.started_at).getTime())
        : formatMilliseconds(log.duration);
    const metrics = [
        { key: 'time', Icon: Clock, iconClassName: 'size-3.5 shrink-0', iconStyle: { color: brandColor } as CSSProperties, value: formatTime(log.started_at), valueClassName: 'tabular-nums', cellClassName: 'col-span-4 whitespace-nowrap md:col-span-1' },
        { key: 'duration', Icon: Cpu, iconClassName: 'size-3.5 shrink-0 text-blue-500', value: duration, cellClassName: 'col-span-4 md:col-span-1' },
        { key: 'cost', Icon: DollarSign, iconClassName: 'size-3.5 shrink-0 text-emerald-500', value: log.cost.toFixed(6), valueClassName: 'font-medium text-emerald-600 dark:text-emerald-400', cellClassName: 'col-span-4 md:col-span-1' },
        { key: 'prompt', Icon: ArrowDownToLine, iconClassName: 'size-3.5 shrink-0 text-green-500', value: (log.usage.prompt_tokens - cachedTokens).toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
        { key: 'cached', Icon: Database, iconClassName: 'size-3.5 shrink-0 text-cyan-500', value: cachedTokens.toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
        { key: 'completion', Icon: ArrowUpFromLine, iconClassName: 'size-3.5 shrink-0 text-purple-500', value: log.usage.completion_tokens.toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
        { key: 'cacheWrite', Icon: Database, iconClassName: 'size-3.5 shrink-0 text-orange-500', value: (log.usage.cache_write_tokens ?? 0).toLocaleString(), cellClassName: 'col-span-3 md:col-span-1' },
    ];

    return metrics.map((metric) => (
        <div key={metric.key} className={cn('flex items-center gap-1.5', variant === 'card' && metric.cellClassName)}>
            <metric.Icon className={metric.iconClassName} style={metric.iconStyle} />
            <span className={metric.valueClassName}>{metric.value}</span>
        </div>
    ));
}

// ObservedRound 保存弹窗打开期间观察到的一轮上游请求状态。
interface ObservedRound {
    round: number; // 当前请求内递增的轮次序号。
    channel: string; // 本轮实际请求的渠道名称。
    error: string; // 本轮最近一次上游错误。
    sending: boolean; // 本轮是否仍在等待上游响应。
}

// JsonContent 渲染请求或响应正文, 能解析为 JSON 时使用折叠视图, 否则按纯文本展示。
function JsonContent({ content, fallbackText }: { content: string | object | undefined; fallbackText: string }) {
    const { resolvedTheme } = useTheme();

    const parsed = useMemo(() => {
        if (content === undefined || content === '') return null;
        if (typeof content !== 'string') return { isJson: true, data: content };
        try {
            return { isJson: true, data: JSON.parse(content) as object };
        } catch {
            return { isJson: false, data: content };
        }
    }, [content]);

    if (!parsed) {
        return (
            <pre className="p-4 text-xs text-muted-foreground whitespace-pre-wrap wrap-break-word leading-relaxed">
                {fallbackText}
            </pre>
        );
    }

    if (!parsed.isJson) {
        return (
            <pre className="p-4 text-xs text-muted-foreground whitespace-pre-wrap wrap-break-word font-mono leading-relaxed select-text animate-in fade-in duration-200">
                {parsed.data as string}
            </pre>
        );
    }

    return (
        <div className="p-4 animate-in fade-in duration-200">
            <JsonView
                value={parsed.data as object}
                style={{
                    ...(resolvedTheme === 'dark' ? githubDarkTheme : githubLightTheme),
                    fontSize: '12px',
                    fontFamily: 'ui-monospace, SFMono-Regular, "SF Mono", Menlo, Consolas, monospace',
                    backgroundColor: 'transparent',
                }}
                displayDataTypes={false}
                displayObjectSize={false}
                collapsed={false}
            />
        </div>
    );
}

// HttpMeta 展示持久化日志的 URL 与 Header, 实时日志(detail 尚未入库)时静默隐藏。
function HttpMeta({ url, headers, loading }: { url?: string; headers?: string; loading: boolean }) {
    const t = useTranslations('log.card');
    if (!url && !headers) return null;

    let parsedHeaders: object | null = null;
    if (headers) {
        try {
            parsedHeaders = JSON.parse(headers) as object;
        } catch {
            parsedHeaders = null;
        }
    }

    return (
        <div className="shrink-0 space-y-1.5 border-b border-border bg-muted/40 px-3 py-2">
            {url && (
                <div className="break-all text-[11px] leading-relaxed text-muted-foreground">
                    <span className="font-semibold text-foreground">{t('url')}: </span>
                    {url}
                </div>
            )}
            {parsedHeaders && (
                <details className="text-[11px]">
                    <summary className="cursor-pointer select-none text-muted-foreground transition-colors hover:text-foreground">
                        {t('headers')}
                    </summary>
                    <pre className="mt-1.5 whitespace-pre-wrap wrap-break-word font-mono leading-relaxed text-muted-foreground">
                        {JSON.stringify(parsedHeaders, null, 2)}
                    </pre>
                </details>
            )}
            {loading && <Loader2 className="size-3.5 animate-spin text-muted-foreground/60" />}
        </div>
    );
}

// LogDetail 渲染日志详情弹窗内容, 仅在弹窗打开期间挂载, 由此避免列表中的卡片持有详情查询和状态。
function LogDetail({ log, now }: { log: RelayLogOverview; now: number }) {
    const t = useTranslations('log.card');
    const statusT = useTranslations('log.status');
    const [leftTab, setLeftTab] = useState<'request' | 'group'>('group');
    const [rounds, setRounds] = useState<ObservedRound[]>([]);
    const [observedRoundKey, setObservedRoundKey] = useState(''); // observedRoundKey 是已记入 rounds 的最近一次日志快照, 用于跳过重复渲染。
    const [detailReady, setDetailReady] = useState(false); // 展开动画结束后才允许加载详情数据。
    const [switchingItemId, setSwitchingItemId] = useState<number | null>(null);
    const requestBody = useLogRequestBody(log.id, log.started_at, detailReady && leftTab === 'request');
    const responseBody = useLogResponseBody(log.id, log.started_at, detailReady && log.status === 'success');
    const detail = useLogDetail(log.id, detailReady);
    const { data: groups = [] } = useGroupList(detailReady, detailReady);
    const { data: channels = [] } = useChannelList(detailReady);
    const updateActiveItem = useUpdateGroupActiveItem();
    const stopRound = useStopRound();
    const channelNameByModelID = useMemo(() => {
        const map = new Map<number, string>();
        channels.forEach(({ raw: channel }) => {
            channel.models.forEach((channelModel) => map.set(channelModel.id, channel.name));
        });
        return map;
    }, [channels]);

    const persistentRounds = useMemo<RoundLog[]>(() => {
        if (log.rounds && log.rounds.length > 0) return log.rounds;
        if (detail.data?.rounds && detail.data.rounds.length > 0) return detail.data.rounds;
        if (detail.data?.rounds_detail) {
            try {
                return JSON.parse(detail.data.rounds_detail) as RoundLog[];
            } catch {
                return [];
            }
        }
        return [];
    }, [log.rounds, detail.data]);

    const [selectedRoundNum, setSelectedRoundNum] = useState<number | null>(null);
    const activeRound = useMemo(() => {
        if (persistentRounds.length === 0) return null;
        if (selectedRoundNum !== null) {
            return persistentRounds.find((r) => r.round === selectedRoundNum) || persistentRounds[persistentRounds.length - 1];
        }
        return persistentRounds[persistentRounds.length - 1];
    }, [persistentRounds, selectedRoundNum]);

    const actualModel = activeRound?.model_name || log.target_model || log.model;
    const actualChannel = activeRound?.channel_name || log.target_channel;
    const { Icon, className: iconClassName, color: brandColor } = getModelIcon(actualModel);
    const errorText = activeRound?.error ?? log.error ?? '';
    const requestFailed = log.status === 'failed' || log.status === 'canceled';
    const responseCommitted = log.status === 'committed';
    const showRounds = log.status === 'running' && rounds.length > 0;
    const activeGroup = groups.find((group) => group.name.toLowerCase() === log.model?.toLowerCase());
    const isWaitingForSelection = log.status === 'running' && !log.sending && activeGroup?.mode === 'manual' && activeGroup.active_item_id === 0;

    // 让弹窗先完成展开动画, 避免详情请求及其状态更新占用动画起步帧。
    useEffect(() => {
        const timer = window.setTimeout(() => setDetailReady(true), 600);
        return () => window.clearTimeout(timer);
    }, []);

    // 按轮次记录本次打开期间观察到的上游请求状态, 最新一轮排在最前。
    const roundKey = log.round === 0 ? '' : `${log.round}:${log.target_channel}:${log.sending}:${log.error ?? ''}`;
    if (roundKey !== '' && roundKey !== observedRoundKey) {
        setObservedRoundKey(roundKey);
        setRounds((current) => {
            if (!log.sending && current.every((item) => item.round !== log.round)) return current;
            return [
                { round: log.round, channel: log.target_channel, error: log.error ?? '', sending: log.sending },
                ...current.filter((item) => item.round !== log.round),
            ];
        });
    }
    return (
        <MorphingDialogContent className="relative w-[calc(100vw-2rem)] md:w-[80vw] bg-card text-card-foreground px-6 py-4 rounded-3xl h-[calc(100vh-2rem)] flex flex-col overflow-hidden">
            <MorphingDialogClose className="top-4 right-5 text-muted-foreground hover:text-foreground transition-colors" />
            <MorphingDialogTitle className="flex items-center gap-2 mb-3 text-sm">
                <Icon aria-hidden="true" className={iconClassName} width={28} height={28} />
                <span className="font-semibold text-card-foreground">{log.model || t('unknownModel')}</span>
                {log.status === 'running' || responseCommitted
                    ? <Loader2 className="size-3.5 animate-spin text-muted-foreground/50" />
                    : <ArrowRight className="size-3.5 text-muted-foreground/50" />}
                <Badge
                    variant="secondary"
                    className="text-xs px-1.5 py-0"
                    style={{ backgroundColor: `${brandColor}15`, color: brandColor }}
                >
                    {actualChannel || '-'}
                </Badge>
                <span className="text-muted-foreground">{actualModel}</span>
                {persistentRounds.length > 1 && (
                    <Badge variant="outline" className="text-[11px] ml-auto">
                        {t('roundIndex', { index: activeRound?.round ?? log.round })} / {persistentRounds.length}
                    </Badge>
                )}
            </MorphingDialogTitle>
            <MorphingDialogDescription className="flex-1 min-h-0 flex flex-col gap-2">
                {persistentRounds.length > 1 && (
                    <div className="flex items-center gap-1.5 overflow-x-auto pb-1 shrink-0">
                        <span className="text-xs font-semibold text-muted-foreground mr-1 shrink-0">{t('round')}:</span>
                        {persistentRounds.map((r) => {
                            const isSelected = activeRound?.round === r.round;
                            const isSuccess = r.status >= 200 && r.status < 300;
                            return (
                                <button
                                    key={r.round}
                                    type="button"
                                    onClick={() => setSelectedRoundNum(r.round)}
                                    className={cn(
                                        'flex items-center gap-1.5 px-2.5 py-1 rounded-lg text-xs font-medium transition-colors border shrink-0',
                                        isSelected
                                            ? 'bg-primary text-primary-foreground border-primary shadow-xs'
                                            : 'bg-muted/50 text-muted-foreground border-border hover:bg-muted hover:text-foreground',
                                    )}
                                >
                                    <span>{t('roundIndex', { index: r.round })}:</span>
                                    <span className="font-semibold">{r.channel_name}</span>
                                    <Badge
                                        variant={isSuccess ? 'secondary' : 'destructive'}
                                        className={cn('px-1 py-0 text-[10px]', isSelected && isSuccess && 'bg-primary-foreground/20 text-primary-foreground')}
                                    >
                                        {r.status || (isSuccess ? 200 : 500)}
                                    </Badge>
                                    {r.duration_ms > 0 && <span className="opacity-70 text-[10px]">{formatMilliseconds(r.duration_ms)}</span>}
                                </button>
                            );
                        })}
                    </div>
                )}
                <div className="grid grid-cols-1 md:grid-cols-2 gap-4 flex-1 min-h-0">
                    <div className="flex flex-col rounded-2xl border border-border bg-muted/30 overflow-hidden min-h-0">
                        <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-muted/50 pl-1 pr-3 md:pr-4">
                            <Tabs value={leftTab} onValueChange={(value) => setLeftTab(value as 'request' | 'group')}>
                                <TabsList variant="text" className="p-0">
                                    <TabsTrigger value="group" className="pr-0">
                                        {t('group')}
                                    </TabsTrigger>
                                    <span aria-hidden="true" className="mx-1 inline-flex h-full -translate-y-px items-center text-sm font-medium leading-none text-muted-foreground/50">/</span>
                                    <TabsTrigger value="request" className="pl-0">
                                        {t('requestContent')}
                                    </TabsTrigger>
                                </TabsList>
                            </Tabs>
                            {leftTab === 'request' && (
                                <Badge variant="secondary" className="ml-auto text-xs">
                                    {(log.usage.prompt_tokens - (log.usage.cache_read_tokens ?? 0)).toLocaleString()} {t('tokens')}
                                </Badge>
                            )}
                        </div>
                        <div className="flex-1 overflow-auto min-h-0">
                            {!detailReady ? (
                                <div className="flex h-full items-center justify-center">
                                    <Loader2 className="size-5 animate-spin text-muted-foreground" />
                                </div>
                            ) : leftTab === 'request' ? (
                                requestBody.isLoading ? (
                                    <div className="flex h-full items-center justify-center">
                                        <Loader2 className="size-5 animate-spin text-muted-foreground" />
                                    </div>
                                ) : requestBody.error ? (
                                    <div className="flex h-full flex-col items-center justify-center gap-2 px-4 text-xs text-destructive">
                                        <AlertCircle className="size-5" />
                                        <span>{t('detailUnavailable')}</span>
                                    </div>
                                ) : (
                                    <>
                                        <HttpMeta
                                            url={activeRound?.request_url || detail.data?.request_url}
                                            headers={activeRound?.request_headers || detail.data?.request_headers}
                                            loading={detail.isLoading}
                                        />
                                        <JsonContent
                                            content={activeRound?.request_body || requestBody.data}
                                            fallbackText={t('noRequestContent')}
                                        />
                                    </>
                                )
                            ) : !activeGroup ? (
                                <div className="flex h-full items-center justify-center px-4 text-xs text-muted-foreground">
                                    {t('groupUnavailable')}
                                </div>
                            ) : !activeGroup.items?.length ? (
                                <div className="flex h-full items-center justify-center px-4 text-xs text-muted-foreground">
                                    {t('noGroupItems')}
                                </div>
                            ) : (
                                <div className="divide-y divide-border">
                                    {activeGroup.items.map((item) => {
                                        const modelName = item.channel_model?.name ?? '';
                                        const channelName = item.channel_model
                                            ? channelNameByModelID.get(item.channel_model.id)
                                                ?? channels.find((c) => c.raw.id === item.channel_model?.channel_id)?.raw.name
                                                ?? `#${item.channel_model.channel_id}`
                                            : '-';
                                        const { Icon: ItemIcon, className: itemIconClassName } = getModelIcon(modelName);
                                        const itemActive = item.id === activeGroup.active_item_id;
                                        const itemSwitching = item.id === switchingItemId;
                                        const isUsedByLog = Boolean(
                                            actualChannel &&
                                            (channelName === actualChannel || (!channelNameByModelID.size && (!channelName || channelName === '-'))) &&
                                            (!actualModel || modelName === actualModel)
                                        );
                                        const itemCurrent = switchingItemId !== null
                                            ? itemSwitching
                                            : isUsedByLog
                                                ? true
                                                : log.status === 'running'
                                                    ? activeGroup.mode === 'failover'
                                                        ? activeGroup.runtime?.current_item_id === item.id
                                                        : itemActive
                                                    : false;
                                        return (
                                            <button
                                                key={item.id ?? item.channel_model_id}
                                                type="button"
                                                aria-pressed={itemCurrent}
                                                disabled={item.id === undefined || activeGroup.mode === 'failover' || switchingItemId !== null || stopRound.isPending}
                                                onClick={async () => {
                                                    if (!activeGroup.id || item.id === undefined || activeGroup.mode === 'failover') return;
                                                    setSwitchingItemId(item.id);
                                                    try {
                                                        await updateActiveItem.mutateAsync({ groupId: activeGroup.id, itemId: itemActive ? 0 : item.id });
                                                        if (log.sending) {
                                                            await stopRound.mutateAsync({ requestId: log.id, round: log.round });
                                                        }
                                                        toast.success(itemActive ? t('channelCleared') : t('channelChanged'));
                                                    } catch (cause) {
                                                        toast.error(t('channelChangeFailed'), { description: cause instanceof Error ? cause.message : undefined });
                                                    } finally {
                                                        setSwitchingItemId(null);
                                                    }
                                                }}
                                                className={cn(
                                                    "flex w-full items-center gap-2.5 rounded-lg px-3 py-2.5 text-left text-xs transition-colors hover:bg-muted/50 disabled:cursor-default disabled:hover:bg-transparent",
                                                    itemCurrent && "bg-muted/60 font-medium"
                                                )}
                                            >
                                                <ItemIcon aria-hidden="true" className={itemIconClassName} width={20} height={20} />
                                                <span className="min-w-0 flex-1">
                                                    <span className="block truncate font-semibold text-foreground">
                                                        {channelName}
                                                    </span>
                                                    <span className="block truncate text-[11px] text-muted-foreground">{modelName}</span>
                                                </span>
                                                <MemberStatus group={activeGroup} itemId={item.id} now={now} active={itemCurrent} />
                                                {itemSwitching && <Loader2 className="size-4 shrink-0 animate-spin text-muted-foreground" />}
                                            </button>
                                        );
                                    })}
                                </div>
                            )}
                        </div>
                    </div>

                    <div className="flex flex-col rounded-2xl border border-border bg-muted/30 overflow-hidden min-h-0">
                        <div className="flex h-10 shrink-0 items-center gap-2 border-b border-border bg-muted/50 px-3 md:px-4">
                            <span className="text-sm font-medium text-card-foreground">
                                {isWaitingForSelection ? t('waitingChannelSelection') : showRounds ? t('retryDetails') : activeRound?.error ? t('errorInfo') : requestFailed ? t('errorInfo') : t('responseContent')}
                            </span>
                            {log.status === 'running' && log.sending && activeGroup?.mode === 'manual' ? (
                                <button
                                    type="button"
                                    disabled={stopRound.isPending}
                                    onClick={async () => {
                                        try {
                                            await stopRound.mutateAsync({ requestId: log.id, round: log.round });
                                        } catch (cause) {
                                            toast.error(t('stopFailed'), { description: cause instanceof Error ? cause.message : undefined });
                                        }
                                    }}
                                    className="ml-auto flex items-center gap-1.5 rounded-md px-2 py-1 text-xs text-destructive transition-colors hover:bg-destructive/10 disabled:opacity-50"
                                >
                                    {stopRound.isPending ? <Loader2 className="size-3.5 animate-spin" /> : <Square className="size-3.5" />}
                                    {t('stopRound')}
                                </button>
                            ) : !requestFailed && (
                                <Badge variant="secondary" className="ml-auto text-xs">
                                    {responseCommitted
                                        ? statusT('committed')
                                        : `${log.usage.completion_tokens.toLocaleString()} ${t('tokens')}`}
                                </Badge>
                            )}
                        </div>
                        <div className="min-h-0 flex-1 overflow-auto">
                            {!detailReady ? (
                                <div className="flex h-full items-center justify-center">
                                    <Loader2 className="size-5 animate-spin text-muted-foreground" />
                                </div>
                            ) : isWaitingForSelection ? (
                                <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
                                    <Loader2 className="size-4 animate-spin" />
                                    {t('waitingChannelSelection')}
                                </div>
                            ) : showRounds ? (
                                rounds.length ? (
                                    <div className="divide-y divide-border">
                                        {rounds.map((round) => (
                                            <div key={round.round} className="flex flex-col gap-1.5 px-3 py-2.5 text-xs">
                                                <div className="flex items-center gap-2">
                                                    <span className="text-muted-foreground">{t('retryIndex', { index: round.round })}</span>
                                                    <span className="font-semibold text-foreground">{round.channel || '-'}</span>
                                                    {round.sending ? (
                                                        <Loader2 className="ml-auto size-3.5 animate-spin text-muted-foreground" />
                                                    ) : round.error ? (
                                                        <CopyIconButton
                                                            text={round.error}
                                                            className="ml-auto p-1 rounded-md text-destructive/60 hover:text-destructive hover:bg-destructive/10 transition-colors"
                                                            copyIconClassName="size-3.5"
                                                            checkIconClassName="size-3.5"
                                                        />
                                                    ) : null}
                                                </div>
                                                {round.error && (
                                                    <div className="text-[11px] leading-relaxed text-destructive/90 whitespace-pre-wrap wrap-break-word">
                                                        {round.error}
                                                    </div>
                                                )}
                                            </div>
                                        ))}
                                    </div>
                                ) : (
                                    <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
                                        <Loader2 className="size-4 animate-spin" />
                                        {t('waitingResponse')}
                                    </div>
                                )
                            ) : responseCommitted ? (
                                <div className="flex h-full items-center justify-center gap-2 text-xs text-muted-foreground">
                                    <Loader2 className="size-4 animate-spin" />
                                    {t('responseStreaming')}
                                </div>
                            ) : activeRound ? (
                                <>
                                    <HttpMeta
                                        url={activeRound.response_url || detail.data?.response_url}
                                        headers={activeRound.response_headers || detail.data?.response_headers}
                                        loading={detail.isLoading}
                                    />
                                    <JsonContent
                                        content={activeRound.response_body || activeRound.error || errorText}
                                        fallbackText={t('noResponseContent')}
                                    />
                                </>
                            ) : requestFailed ? (
                                <JsonContent content={errorText} fallbackText={t('noResponseContent')} />
                            ) : responseBody.isLoading ? (
                                <div className="flex h-full items-center justify-center">
                                    <Loader2 className="size-5 animate-spin text-muted-foreground" />
                                </div>
                            ) : responseBody.error ? (
                                <div className="flex h-full flex-col items-center justify-center gap-2 px-4 text-xs text-destructive">
                                    <AlertCircle className="size-5" />
                                    <span>{t('detailUnavailable')}</span>
                                </div>
                            ) : (
                                <>
                                    <HttpMeta
                                        url={detail.data?.response_url}
                                        headers={detail.data?.response_headers}
                                        loading={detail.isLoading}
                                    />
                                    <JsonContent content={responseBody.data} fallbackText={t('noResponseContent')} />
                                </>
                            )}
                        </div>
                    </div>
                </div>
            </MorphingDialogDescription>

            <div className="flex w-full shrink-0 flex-wrap items-center gap-3 pt-4 mt-auto text-xs text-muted-foreground md:gap-4">
                <LogMetrics log={log} now={now} brandColor={brandColor} variant="footer" />
            </div>
        </MorphingDialogContent>
    );
}

// LogCardBody 渲染日志概览卡片, 并在弹窗打开时挂载详情面板。
function LogCardBody({ log }: { log: RelayLogOverview }) {
    const t = useTranslations('log.card');
    const { isOpen } = useMorphingDialog();
    const [now, setNow] = useState(() => Date.now());
    const actualModel = log.target_model || log.model;
    const { Icon, className: iconClassName, color: brandColor } = getModelIcon(actualModel);
    const requestRunning = log.status === 'running' || log.status === 'committed';
    const requestFailed = log.status === 'failed' || log.status === 'canceled';
    const errorText = log.error ?? '';

    // 仅在请求进行中或弹窗打开时走秒级刷新, 避免已完成日志持续触发重渲染。
    useEffect(() => {
        if (!requestRunning && !isOpen) return;
        const timer = window.setInterval(() => setNow(Date.now()), 1000);
        return () => window.clearInterval(timer);
    }, [isOpen, requestRunning]);

    return (
        <>
            <MorphingDialogTrigger
                className={cn(
                    "rounded-3xl border bg-card w-full text-left",
                    requestFailed ? "border-destructive/40" : "border-border",
                )}
            >
                <div className={cn("p-4 grid grid-cols-[auto_1fr] gap-4", requestFailed ? "items-start" : "items-center")}>
                    <Icon aria-hidden="true" className={iconClassName} width={40} height={40} />
                    <div className="min-w-0 flex flex-col gap-3">
                        <div className="flex items-center gap-2 min-w-0 text-sm">
                            <span className="font-semibold text-card-foreground truncate" title={log.model}>
                                {log.model || t('unknownModel')}
                            </span>
                            {requestRunning
                                ? <Loader2 className="size-3.5 shrink-0 animate-spin text-muted-foreground/50" />
                                : <ArrowRight className="size-3.5 shrink-0 text-muted-foreground/50" />}
                            <Badge
                                variant="secondary"
                                className="shrink-0 text-xs px-1.5 py-0"
                                style={{ backgroundColor: `${brandColor}15`, color: brandColor }}
                            >
                                {log.target_channel || '-'}
                            </Badge>
                            <span className="text-muted-foreground truncate" title={actualModel}>
                                {actualModel}
                            </span>
                        </div>
                        <div className="grid grid-cols-12 gap-x-4 gap-y-2 text-xs tabular-nums text-muted-foreground md:grid-cols-7">
                            <LogMetrics log={log} now={now} brandColor={brandColor} variant="card" />
                        </div>
                        {requestFailed && errorText && (
                            <div className="p-2.5 rounded-xl bg-destructive/10 border border-destructive/20 overflow-hidden">
                                <p className="text-xs text-destructive line-clamp-2 whitespace-pre-line">{errorText}</p>
                            </div>
                        )}
                    </div>
                </div>
            </MorphingDialogTrigger>

            <MorphingDialogContainer>
                <LogDetail log={log} now={now} />
            </MorphingDialogContainer>
        </>
    );
}

// LogCard 展示一条日志概览, 并在弹窗打开时加载详情。
export const LogCard = memo(function LogCard({ log }: { log: RelayLogOverview }) {
    return (
        <MorphingDialog>
            <LogCardBody log={log} />
        </MorphingDialog>
    );
});
