import { useRef, useState, type ReactNode } from 'react';
import { create } from 'zustand';
import { persist } from 'zustand/middleware';
import { ArrowUpAZ, LayoutGrid, List, Plus, Search, SlidersHorizontal, X } from 'lucide-react';
import { useTranslations } from 'use-intl';
import {
    MorphingDialog,
    MorphingDialogContainer,
    MorphingDialogContent,
    MorphingDialogTrigger,
} from '@/components/ui/morphing-dialog';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';


type PageActionPage = 'channel' | 'group' | 'model';
type PageActionLayout = 'grid' | 'list';
type PageActionSortOrder = 'asc' | 'desc';
type ChannelFilter = 'all' | 'enabled' | 'disabled';
type GroupFilter = 'all' | 'with-members' | 'empty';
type ModelFilter = 'all' | 'priced' | 'free';

interface PageActionsState {
    searchTerms: Partial<Record<PageActionPage, string>>; // 各列表页面当前的搜索内容。
    layouts: Partial<Record<PageActionPage, PageActionLayout>>; // 各列表页面选中的布局。
    sortOrders: Partial<Record<PageActionPage, PageActionSortOrder>>; // 各列表页面的排序方向。
    channelFilter: ChannelFilter; // 渠道页面的筛选条件。
    groupFilter: GroupFilter; // 分组页面的筛选条件。
    modelFilter: ModelFilter; // 模型页面的筛选条件。
    setSearchTerm: (page: PageActionPage, value: string) => void; // 更新指定页面的搜索内容。
    setLayout: (page: PageActionPage, value: PageActionLayout) => void; // 更新指定页面的布局。
    setSort: (page: PageActionPage, order: PageActionSortOrder) => void; // 更新指定页面的排序方向。
    setChannelFilter: (value: ChannelFilter) => void; // 更新渠道筛选条件。
    setGroupFilter: (value: GroupFilter) => void; // 更新分组筛选条件。
    setModelFilter: (value: ModelFilter) => void; // 更新模型筛选条件。
}

// usePageActionsStore 保存稳定顶栏与页面正文共享的视图选项。
export const usePageActionsStore = create<PageActionsState>()(
    persist(
        (set) => ({
            searchTerms: {},
            layouts: {},
            sortOrders: {},
            channelFilter: 'all',
            groupFilter: 'all',
            modelFilter: 'all',
            setSearchTerm: (page, value) => set((state) => ({
                searchTerms: { ...state.searchTerms, [page]: value },
            })),
            setLayout: (page, value) => set((state) => ({
                layouts: { ...state.layouts, [page]: value },
            })),
            setSort: (page, order) => set((state) => ({
                sortOrders: { ...state.sortOrders, [page]: order },
            })),
            setChannelFilter: (value) => set({ channelFilter: value }),
            setGroupFilter: (value) => set({ groupFilter: value }),
            setModelFilter: (value) => set({ modelFilter: value }),
        }),
        {
            name: 'page-actions-storage',
            partialize: (state) => ({
                layouts: state.layouts,
                sortOrders: state.sortOrders,
                channelFilter: state.channelFilter,
                groupFilter: state.groupFilter,
                modelFilter: state.modelFilter,
            }),
        }
    )
);

interface PageActionsProps {
    page: PageActionPage;
    children: ReactNode;
}

// PageActions owns the three list-page presets; callers only identify the page and provide create content.
export function PageActions({ page, children }: PageActionsProps) {
    const t = useTranslations('toolbar');
    const searchTerm = usePageActionsStore((state) => state.searchTerms[page] || '');
    const layout = usePageActionsStore((state) => state.layouts[page] || 'grid');
    const sortValue = usePageActionsStore((state) => state.sortOrders[page] === 'desc' ? 'desc' : 'asc');
    const setSearchTerm = usePageActionsStore((state) => state.setSearchTerm);
    const setLayout = usePageActionsStore((state) => state.setLayout);
    const setSort = usePageActionsStore((state) => state.setSort);
    const channelFilter = usePageActionsStore((state) => state.channelFilter);
    const groupFilter = usePageActionsStore((state) => state.groupFilter);
    const modelFilter = usePageActionsStore((state) => state.modelFilter);
    const setChannelFilter = usePageActionsStore((state) => state.setChannelFilter);
    const setGroupFilter = usePageActionsStore((state) => state.setGroupFilter);
    const setModelFilter = usePageActionsStore((state) => state.setModelFilter);
    const filterValue = page === 'channel' ? channelFilter : page === 'group' ? groupFilter : modelFilter;
    const filterOptions = page === 'channel'
        ? [
            { value: 'all', label: t('popover.filter.channel.all') },
            { value: 'enabled', label: t('popover.filter.channel.enabled') },
            { value: 'disabled', label: t('popover.filter.channel.disabled') },
        ]
        : page === 'group'
            ? [
                { value: 'all', label: t('popover.filter.group.all') },
                { value: 'with-members', label: t('popover.filter.group.withMembers') },
                { value: 'empty', label: t('popover.filter.group.empty') },
            ]
            : [
                { value: 'all', label: t('popover.filter.model.all') },
                { value: 'priced', label: t('popover.filter.model.priced') },
                { value: 'free', label: t('popover.filter.model.free') },
            ];
    const setFilter = (value: string) => {
        if (page === 'channel' && (value === 'all' || value === 'enabled' || value === 'disabled')) setChannelFilter(value);
        if (page === 'group' && (value === 'all' || value === 'with-members' || value === 'empty')) setGroupFilter(value);
        if (page === 'model' && (value === 'all' || value === 'priced' || value === 'free')) setModelFilter(value);
    };
    const sortOptions = [
        { value: 'asc', label: t('popover.nameAsc'), icon: ArrowUpAZ },
        { value: 'desc', label: t('popover.nameDesc'), icon: ArrowUpAZ },
    ];
    const [searchExpanded, setSearchExpanded] = useState(false);
    const searchInputRef = useRef<HTMLInputElement>(null);

    return (
        <div className="flex items-center gap-2">
            {/* 搜索按钮/展开框 */}
            <div className="relative h-9 w-9">
                <div
                    className={cn(
                        'absolute right-0 top-0 flex h-9 items-center gap-2 overflow-hidden rounded-xl border transition-[width,padding,border-color] duration-200 ease-linear',
                        searchExpanded ? 'w-39 border-border px-3' : 'w-9 border-transparent px-0',
                    )}
                >
                    <button
                        type="button"
                        onClick={() => {
                            if (searchExpanded) return;
                            setSearchExpanded(true);
                            window.requestAnimationFrame(() => searchInputRef.current?.focus());
                        }}
                        className={cn(
                            'flex h-full shrink-0 items-center justify-center text-muted-foreground transition-[width,color] duration-200 ease-linear hover:text-foreground',
                            searchExpanded ? 'w-4 cursor-default' : 'w-9',
                        )}
                    >
                        <Search className="size-4" />
                    </button>
                    <input
                        ref={searchInputRef}
                        type="text"
                        value={searchTerm}
                        onChange={(event) => setSearchTerm(page, event.target.value)}
                        tabIndex={searchExpanded ? 0 : -1}
                        className="w-20 shrink-0 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
                    />
                    <button
                        type="button"
                        tabIndex={searchExpanded ? 0 : -1}
                        onClick={() => {
                            setSearchTerm(page, '');
                            setSearchExpanded(false);
                        }}
                        className="shrink-0 rounded p-0.5 text-muted-foreground transition-colors hover:text-foreground"
                    >
                        <X className="size-3.5" />
                    </button>
                </div>
            </div>

            <Popover>
                <PopoverTrigger asChild>
                    <button
                        type="button"
                        aria-label={t('popover.ariaLabel')}
                        className={buttonVariants({
                            variant: 'ghost',
                            size: 'icon',
                            className: 'rounded-xl transition-none hover:bg-transparent text-muted-foreground hover:text-foreground',
                        })}
                    >
                        <SlidersHorizontal className="size-4 transition-colors duration-300" />
                    </button>
                </PopoverTrigger>
                <PopoverContent
                    align="center"
                    side="bottom"
                    sideOffset={8}
                    className="w-64 rounded-2xl border border-border/60 bg-card p-3 shadow-xl"
                >
                    <div className="grid gap-3">
                        {page !== 'group' && (
                            <div className="grid gap-2">
                                <p className="text-xs font-medium text-muted-foreground">{t('popover.layout')}</p>
                                <div className="grid grid-cols-2 gap-2">
                                    <button
                                        type="button"
                                        onClick={() => setLayout(page, 'grid')}
                                        className={cn(
                                            'inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border text-xs font-medium transition-colors',
                                            layout === 'grid'
                                                ? 'border-primary/30 bg-primary text-primary-foreground'
                                                : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                        )}
                                    >
                                        <LayoutGrid className="size-3.5" />
                                        {t('popover.grid')}
                                    </button>
                                    <button
                                        type="button"
                                        onClick={() => setLayout(page, 'list')}
                                        className={cn(
                                            'inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border text-xs font-medium transition-colors',
                                            layout === 'list'
                                                ? 'border-primary/30 bg-primary text-primary-foreground'
                                                : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                        )}
                                    >
                                        <List className="size-3.5" />
                                        {t('popover.list')}
                                    </button>
                                </div>
                            </div>
                        )}

                        <div className="grid gap-2">
                            <p className="text-xs font-medium text-muted-foreground">{t('popover.sort')}</p>
                            <div className="grid grid-cols-2 gap-2">
                                {sortOptions.map((option) => {
                                    const Icon = option.icon;
                                    return (
                                        <button
                                            key={option.value}
                                            type="button"
                                            onClick={() => setSort(page, option.value === 'desc' ? 'desc' : 'asc')}
                                            className={cn(
                                                'inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border text-xs font-medium transition-colors',
                                                sortValue === option.value
                                                    ? 'border-primary/30 bg-primary text-primary-foreground'
                                                    : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                            )}
                                        >
                                            {Icon && <Icon className="size-3.5" />}
                                            {option.label}
                                        </button>
                                    );
                                })}
                            </div>
                        </div>

                        <div className="grid gap-2">
                            <p className="text-xs font-medium text-muted-foreground">{t('popover.filter.title')}</p>
                            <div className="grid gap-2">
                                {filterOptions.map((option) => (
                                    <button
                                        key={option.value}
                                        type="button"
                                        onClick={() => setFilter(option.value)}
                                        className={cn(
                                            'h-8 rounded-lg border px-2 text-left text-xs font-medium transition-colors',
                                            filterValue === option.value
                                                ? 'border-primary/30 bg-primary text-primary-foreground'
                                                : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                        )}
                                    >
                                        {option.label}
                                    </button>
                                ))}
                            </div>
                        </div>
                    </div>
                </PopoverContent>
            </Popover>

            {/* 创建按钮 */}
            <MorphingDialog>
                <MorphingDialogTrigger className={buttonVariants({ variant: 'ghost', size: 'icon', className: 'rounded-xl transition-none hover:bg-transparent text-muted-foreground hover:text-foreground' })}>
                    <Plus className="size-4 transition-colors duration-300" />
                </MorphingDialogTrigger>
                <MorphingDialogContainer>
                    <MorphingDialogContent className="flex max-h-[calc(100vh-2rem)] w-fit max-w-full flex-col overflow-hidden rounded-3xl bg-card px-6 py-4 text-card-foreground">
                        {children}
                    </MorphingDialogContent>
                </MorphingDialogContainer>
            </MorphingDialog>
        </div>
    );
}
