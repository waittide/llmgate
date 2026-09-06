<script setup lang="ts">
import { computed, ref } from 'vue';
import {
    LuLoader2,
    LuRefreshCw,
    LuScrollText,
    LuSearch,
} from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { cn } from '@/lib/utils';
import {
    type PersistentLogsParams,
    type RelayLogOverview,
    useLogListLiveSync,
    usePersistentLogs,
} from '@/api/log';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from '@/components/ui/select';
import VirtualizedGrid from '@/components/common/VirtualizedGrid.vue';
import LogCard from './Item.vue';

const PAGE_SIZE = 50;

interface LogFilters {
    keyword: string;
    status: string;
    model: string;
}

const { t } = useTranslations('log');
useLogListLiveSync();

const page = ref(1);
const keyword = ref('');
const status = ref('all');
const model = ref('');
const applied = ref<LogFilters>({ keyword: '', status: '', model: '' });

const filterParams = computed<PersistentLogsParams>(() => ({
    page: page.value,
    pageSize: PAGE_SIZE,
    keyword: applied.value.keyword,
    status: applied.value.status,
    model: applied.value.model,
}));

const { data, isLoading, isFetching, error, refetch } = usePersistentLogs(filterParams);

const total = computed(() => data.value?.total ?? 0);
const items = computed(() => data.value?.items ?? []);
const totalPages = computed(() => Math.max(1, Math.ceil(total.value / PAGE_SIZE)));

const applyFilters = () => {
    applied.value = {
        keyword: keyword.value.trim(),
        status: status.value === 'all' ? '' : status.value,
        model: model.value.trim(),
    };
    page.value = 1;
};
</script>

<template>
    <div class="flex h-full min-h-0 flex-col gap-3">
        <!-- 过滤工具栏 -->
        <div class="flex shrink-0 flex-wrap items-center gap-2">
            <div class="relative">
                <LuSearch class="pointer-events-none absolute left-3 top-1/2 size-4 -translate-y-1/2 text-muted-foreground" />
                <Input
                    v-model="keyword"
                    @keydown.enter="applyFilters"
                    :placeholder="t('filter.keyword')"
                    class="w-52 rounded-xl pl-9"
                />
            </div>
            <Input
                v-model="model"
                @keydown.enter="applyFilters"
                :placeholder="t('filter.model')"
                class="w-40 rounded-xl"
            />
            <Select v-model="status">
                <SelectTrigger class="w-36 rounded-xl">
                    <SelectValue :placeholder="t('filter.status')" />
                </SelectTrigger>
                <SelectContent>
                    <SelectItem value="all">{{ t('filter.all') }}</SelectItem>
                    <SelectItem value="success">{{ t('filter.success') }}</SelectItem>
                    <SelectItem value="failed">{{ t('filter.failed') }}</SelectItem>
                    <SelectItem value="stream_error">{{ t('filter.streamError') }}</SelectItem>
                </SelectContent>
            </Select>
            <Button variant="secondary" size="sm" class="rounded-xl" @click="applyFilters">
                {{ t('filter.apply') }}
            </Button>
            <Button
                variant="ghost"
                size="icon"
                class="size-9 rounded-xl"
                @click="() => refetch()"
                :disabled="isFetching"
                :title="t('filter.refresh')"
            >
                <LuRefreshCw :class="cn('size-4', isFetching && 'animate-spin')" />
            </Button>
        </div>

        <!-- 日志列表 -->
        <div class="min-h-0 flex-1 pr-1">
            <div v-if="isLoading" class="flex h-full items-center justify-center">
                <LuLoader2 class="size-6 animate-spin text-muted-foreground" />
            </div>
            <div v-else-if="error" class="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
                <span class="text-sm">{{ t('list.disconnected') }}</span>
            </div>
            <div v-else-if="items.length === 0" class="flex h-full flex-col items-center justify-center gap-3 text-muted-foreground">
                <LuScrollText class="size-8" />
                <span class="text-sm">{{ t('list.empty') }}</span>
            </div>
            <VirtualizedGrid
                v-else
                layout="list"
                :columns="{ default: 1 }"
                :items="items"
                :estimate-item-height="116"
                :gap="12"
                :get-item-key="(log: RelayLogOverview) => `log-${log.id}-${log.started_at}`"
            >
                <template #default="{ item }">
                    <LogCard :log="item" />
                </template>
            </VirtualizedGrid>
        </div>

        <!-- 分页 -->
        <div class="flex shrink-0 items-center justify-between border-t border-border pt-3 text-xs text-muted-foreground">
            <span>
                {{ t('pagination.total', { total }) }}
                {{ totalPages > 1 ? ` · ${t('pagination.page', { page, totalPages })}` : '' }}
            </span>
            <div class="flex items-center gap-2">
                <Button
                    variant="outline"
                    size="sm"
                    class="rounded-xl"
                    :disabled="page <= 1 || isFetching"
                    @click="page = Math.max(1, page - 1)"
                >
                    {{ t('pagination.prev') }}
                </Button>
                <Button
                    variant="outline"
                    size="sm"
                    class="rounded-xl"
                    :disabled="page >= totalPages || isFetching"
                    @click="page = Math.min(totalPages, page + 1)"
                >
                    {{ t('pagination.next') }}
                </Button>
            </div>
        </div>
    </div>
</template>
