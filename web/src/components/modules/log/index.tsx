import { useState } from 'react';
import { Loader2, Logs, RefreshCw, Search } from 'lucide-react';
import { useTranslations } from 'use-intl';
import { usePersistentLogs } from '@/api/log';
import { LogCard } from './Item';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Select, SelectContent, SelectItem, SelectTrigger, SelectValue } from '@/components/ui/select';
import { cn } from '@/lib/utils';

const PAGE_SIZE = 50;

interface LogFilters {
    keyword: string;
    status: string;
    model: string;
}

// Log 展示数据库中的持久化请求日志, 支持关键词/状态/模型过滤、刷新与分页。
export function Log() {
    const t = useTranslations('log');
    const [page, setPage] = useState(1);
    const [keyword, setKeyword] = useState('');
    const [status, setStatus] = useState('all');
    const [model, setModel] = useState('');
    const [applied, setApplied] = useState<LogFilters>({ keyword: '', status: '', model: '' });

    const { data, isLoading, isFetching, error, refetch } = usePersistentLogs({
        page,
        pageSize: PAGE_SIZE,
        ...applied,
    });

    const total = data?.total ?? 0;
    const items = data?.items ?? [];
    const totalPages = Math.max(1, Math.ceil(total / PAGE_SIZE));

    const applyFilters = () => {
        setApplied({ keyword: keyword.trim(), status: status === 'all' ? '' : status, model: model.trim() });
        setPage(1);
    };

    return (
        <div className="flex h-full min-h-0 flex-col gap-3">
            {/* 过滤工具栏 */}
            <div className="flex shrink-0 flex-wrap items-center gap-2">
                <div className="relative">
                    <Search className="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                    <Input
                        value={keyword}
                        onChange={(e) => setKeyword(e.target.value)}
                        onKeyDown={(e) => { if (e.key === 'Enter') applyFilters(); }}
                        placeholder={t('filter.keyword')}
                        className="w-52 rounded-xl pl-9"
                    />
                </div>
                <Input
                    value={model}
                    onChange={(e) => setModel(e.target.value)}
                    onKeyDown={(e) => { if (e.key === 'Enter') applyFilters(); }}
                    placeholder={t('filter.model')}
                    className="w-40 rounded-xl"
                />
                <Select value={status} onValueChange={setStatus}>
                    <SelectTrigger className="w-36 rounded-xl">
                        <SelectValue placeholder={t('filter.status')} />
                    </SelectTrigger>
                    <SelectContent>
                        <SelectItem value="all">{t('filter.all')}</SelectItem>
                        <SelectItem value="success">{t('filter.success')}</SelectItem>
                        <SelectItem value="failed">{t('filter.failed')}</SelectItem>
                        <SelectItem value="stream_error">{t('filter.streamError')}</SelectItem>
                    </SelectContent>
                </Select>
                <Button variant="secondary" size="sm" className="rounded-xl" onClick={applyFilters}>
                    {t('filter.apply')}
                </Button>
                <Button
                    variant="ghost"
                    size="icon"
                    className="size-9 rounded-xl"
                    onClick={() => refetch()}
                    disabled={isFetching}
                    title={t('filter.refresh')}
                >
                    <RefreshCw className={cn('size-4', isFetching && 'animate-spin')} />
                </Button>
            </div>

            {/* 日志列表 */}
            <div className="min-h-0 flex-1 overflow-y-auto space-y-3 pr-1">
                {isLoading ? (
                    <div className="flex h-full items-center justify-center">
                        <Loader2 className="size-6 animate-spin text-muted-foreground" />
                    </div>
                ) : error ? (
                    <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
                        <span className="text-sm">{t('list.disconnected')}</span>
                    </div>
                ) : items.length === 0 ? (
                    <div className="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
                        <Logs className="size-8" />
                        <span className="text-sm">{t('list.empty')}</span>
                    </div>
                ) : (
                    items.map((log) => <LogCard key={`log-${log.id}-${log.started_at}`} log={log} />)
                )}
            </div>

            {/* 分页 */}
            <div className="flex shrink-0 items-center justify-between border-t border-border pt-3 text-xs text-muted-foreground">
                <span>
                    {t('pagination.total', { total })}
                    {totalPages > 1 && ` · ${t('pagination.page', { page, totalPages })}`}
                </span>
                <div className="flex items-center gap-2">
                    <Button variant="outline" size="sm" className="rounded-xl" disabled={page <= 1 || isFetching} onClick={() => setPage((p) => Math.max(1, p - 1))}>
                        {t('pagination.prev')}
                    </Button>
                    <Button variant="outline" size="sm" className="rounded-xl" disabled={page >= totalPages || isFetching} onClick={() => setPage((p) => Math.min(totalPages, p + 1))}>
                        {t('pagination.next')}
                    </Button>
                </div>
            </div>
        </div>
    );
}
