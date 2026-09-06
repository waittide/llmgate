<script setup lang="ts">
import { ref, computed, nextTick } from 'vue';
import {
    LuArrowUpAZ,
    LuLayoutGrid,
    LuList,
    LuPlus,
    LuSearch,
    LuSlidersHorizontal,
    LuX,
} from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import {
    MorphingDialog,
    MorphingDialogContainer,
    MorphingDialogContent,
    MorphingDialogTrigger,
} from '@/components/ui/morphing-dialog';
import { Popover, PopoverContent, PopoverTrigger } from '@/components/ui/popover';
import { buttonVariants } from '@/components/ui/button';
import { cn } from '@/lib/utils';
import {
    pageActionsStore,
    type PageActionPage,
    type ChannelFilter,
    type GroupFilter,
    type ModelFilter,
} from './page-actions-store';

export interface PageActionsProps {
    page: PageActionPage;
}

const props = defineProps<PageActionsProps>();

const { t } = useTranslations('toolbar');

const searchTerm = computed({
    get: () => pageActionsStore.searchTerms[props.page] || '',
    set: (val: string) => pageActionsStore.setSearchTerm(props.page, val),
});

const layout = computed({
    get: () => pageActionsStore.layouts[props.page] || 'grid',
    set: (val: 'grid' | 'list') => pageActionsStore.setLayout(props.page, val),
});

const sortValue = computed({
    get: () => (pageActionsStore.sortOrders[props.page] === 'desc' ? 'desc' : 'asc'),
    set: (val: 'asc' | 'desc') => pageActionsStore.setSort(props.page, val),
});

const filterValue = computed(() => {
    if (props.page === 'channel') return pageActionsStore.channelFilter;
    if (props.page === 'group') return pageActionsStore.groupFilter;
    return pageActionsStore.modelFilter;
});

const filterOptions = computed(() => {
    if (props.page === 'channel') {
        return [
            { value: 'all', label: t('popover.filter.channel.all') },
            { value: 'enabled', label: t('popover.filter.channel.enabled') },
            { value: 'disabled', label: t('popover.filter.channel.disabled') },
        ];
    }
    if (props.page === 'group') {
        return [
            { value: 'all', label: t('popover.filter.group.all') },
            { value: 'with-members', label: t('popover.filter.group.withMembers') },
            { value: 'empty', label: t('popover.filter.group.empty') },
        ];
    }
    return [
        { value: 'all', label: t('popover.filter.model.all') },
        { value: 'priced', label: t('popover.filter.model.priced') },
        { value: 'free', label: t('popover.filter.model.free') },
    ];
});

function setFilter(value: string) {
    if (props.page === 'channel' && (value === 'all' || value === 'enabled' || value === 'disabled')) {
        pageActionsStore.setChannelFilter(value as ChannelFilter);
    } else if (props.page === 'group' && (value === 'all' || value === 'with-members' || value === 'empty')) {
        pageActionsStore.setGroupFilter(value as GroupFilter);
    } else if (props.page === 'model' && (value === 'all' || value === 'priced' || value === 'free')) {
        pageActionsStore.setModelFilter(value as ModelFilter);
    }
}

const sortOptions = [
    { value: 'asc', label: t('popover.nameAsc'), icon: LuArrowUpAZ },
    { value: 'desc', label: t('popover.nameDesc'), icon: LuArrowUpAZ },
];

const searchExpanded = ref(false);
const searchInputRef = ref<HTMLInputElement | null>(null);

function handleExpandSearch() {
    if (searchExpanded.value) return;
    searchExpanded.value = true;
    nextTick(() => {
        searchInputRef.value?.focus();
    });
}

function handleCollapseSearch() {
    searchTerm.value = '';
    searchExpanded.value = false;
}
</script>

<template>
    <div class="flex items-center gap-2">
        <!-- 搜索按钮/展开框 -->
        <div class="relative h-9 w-9">
            <div
                :class="cn(
                    'absolute right-0 top-0 flex h-9 items-center gap-2 overflow-hidden rounded-xl border transition-[width,padding,border-color] duration-200 ease-linear',
                    searchExpanded ? 'w-39 border-border px-3' : 'w-9 border-transparent px-0',
                )"
            >
                <button
                    type="button"
                    @click="handleExpandSearch"
                    :class="cn(
                        'flex h-full shrink-0 items-center justify-center text-muted-foreground transition-[width,color] duration-200 ease-linear hover:text-foreground',
                        searchExpanded ? 'w-4 cursor-default' : 'w-9',
                    )"
                >
                    <LuSearch class="size-4" />
                </button>
                <input
                    ref="searchInputRef"
                    type="text"
                    v-model="searchTerm"
                    :tabindex="searchExpanded ? 0 : -1"
                    class="w-20 shrink-0 bg-transparent text-sm outline-none placeholder:text-muted-foreground"
                />
                <button
                    type="button"
                    :tabindex="searchExpanded ? 0 : -1"
                    @click="handleCollapseSearch"
                    class="shrink-0 rounded p-0.5 text-muted-foreground transition-colors hover:text-foreground"
                >
                    <LuX class="size-3.5" />
                </button>
            </div>
        </div>

        <!-- 筛选与布局弹层 -->
        <Popover>
            <PopoverTrigger as-child>
                <button
                    type="button"
                    :aria-label="t('popover.ariaLabel')"
                    :class="buttonVariants({
                        variant: 'ghost',
                        size: 'icon',
                        className: 'rounded-xl transition-none hover:bg-transparent text-muted-foreground hover:text-foreground',
                    })"
                >
                    <LuSlidersHorizontal class="size-4 transition-colors duration-300" />
                </button>
            </PopoverTrigger>
            <PopoverContent
                align="center"
                side="bottom"
                :side-offset="8"
                class="w-64 rounded-2xl border border-border/60 bg-card p-3 shadow-xl"
            >
                <div class="grid gap-3">
                    <div v-if="page !== 'group'" class="grid gap-2">
                        <p class="text-xs font-medium text-muted-foreground">{{ t('popover.layout') }}</p>
                        <div class="grid grid-cols-2 gap-2">
                            <button
                                type="button"
                                @click="layout = 'grid'"
                                :class="cn(
                                    'inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border text-xs font-medium transition-colors',
                                    layout === 'grid'
                                        ? 'border-primary/30 bg-primary text-primary-foreground'
                                        : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                )"
                            >
                                <LuLayoutGrid class="size-3.5" />
                                {{ t('popover.grid') }}
                            </button>
                            <button
                                type="button"
                                @click="layout = 'list'"
                                :class="cn(
                                    'inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border text-xs font-medium transition-colors',
                                    layout === 'list'
                                        ? 'border-primary/30 bg-primary text-primary-foreground'
                                        : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                )"
                            >
                                <LuList class="size-3.5" />
                                {{ t('popover.list') }}
                            </button>
                        </div>
                    </div>

                    <div class="grid gap-2">
                        <p class="text-xs font-medium text-muted-foreground">{{ t('popover.sort') }}</p>
                        <div class="grid grid-cols-2 gap-2">
                            <button
                                v-for="option in sortOptions"
                                :key="option.value"
                                type="button"
                                @click="sortValue = option.value as 'asc' | 'desc'"
                                :class="cn(
                                    'inline-flex h-8 items-center justify-center gap-1.5 rounded-lg border text-xs font-medium transition-colors',
                                    sortValue === option.value
                                        ? 'border-primary/30 bg-primary text-primary-foreground'
                                        : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                )"
                            >
                                <component :is="option.icon" class="size-3.5" />
                                {{ option.label }}
                            </button>
                        </div>
                    </div>

                    <div class="grid gap-2">
                        <p class="text-xs font-medium text-muted-foreground">{{ t('popover.filter.title') }}</p>
                        <div class="grid gap-2">
                            <button
                                v-for="option in filterOptions"
                                :key="option.value"
                                type="button"
                                @click="setFilter(option.value)"
                                :class="cn(
                                    'h-8 rounded-lg border px-2 text-left text-xs font-medium transition-colors',
                                    filterValue === option.value
                                        ? 'border-primary/30 bg-primary text-primary-foreground'
                                        : 'border-border bg-muted/20 text-foreground hover:bg-muted/30'
                                )"
                            >
                                {{ option.label }}
                            </button>
                        </div>
                    </div>
                </div>
            </PopoverContent>
        </Popover>

        <!-- 创建按钮与弹窗 -->
        <MorphingDialog>
            <MorphingDialogTrigger :class="buttonVariants({ variant: 'ghost', size: 'icon', className: 'rounded-xl transition-none hover:bg-transparent text-muted-foreground hover:text-foreground' })">
                <LuPlus class="size-4 transition-colors duration-300" />
            </MorphingDialogTrigger>
            <MorphingDialogContainer>
                <MorphingDialogContent class="flex max-h-[calc(100vh-2rem)] w-fit max-w-full flex-col overflow-hidden rounded-3xl bg-card px-6 py-4 text-card-foreground">
                    <slot />
                </MorphingDialogContent>
            </MorphingDialogContainer>
        </MorphingDialog>
    </div>
</template>
