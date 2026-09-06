<script setup lang="ts">
import { ref, computed, useSlots } from 'vue';
import {
    LuCheck,
    LuHelpCircle,
    LuPlus,
    LuSearch,
    LuSparkles,
    LuTrash2,
} from 'vue-icons-plus/lu';
import { useTranslations } from '@/composables/i18n';
import { useChannelList } from '@/api/channel';
import { Button } from '@/components/ui/button';
import { Field, FieldGroup, FieldLabel } from '@/components/ui/field';
import { Input } from '@/components/ui/input';
import {
    Accordion,
    AccordionContent,
    AccordionItem,
    AccordionTrigger,
} from '@/components/ui/accordion';
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from '@/components/ui/select';
import { Tabs, TabsContent, TabsList, TabsTrigger } from '@/components/ui/tabs';
import { Tooltip, TooltipContent, TooltipTrigger } from '@/components/ui/tooltip';
import { cn } from '@/lib/utils';
import type { GroupMode, GroupRelayConfig } from '@/api/group';
import MemberList, { type SelectedMember } from './ItemList.vue';
import { matchesGroupName, memberKey, normalizeKey } from './utils';
import { getModelIcon } from './model-icons';

export type GroupEditorValues = {
    name: string;
    mode: GroupMode;
    relay_config: GroupRelayConfig;
    members: SelectedMember[];
};

export interface GroupEditorProps {
    initial?: {
        name?: string;
        mode?: GroupMode;
        relay_config?: Partial<GroupRelayConfig>;
        members?: SelectedMember[];
    };
    submitText: string;
    submittingText: string;
    isSubmitting: boolean;
    onCancel?: () => void;
}

const props = withDefaults(defineProps<GroupEditorProps>(), {
    initial: undefined,
    onCancel: undefined,
});

const emit = defineEmits<{
    (e: 'submit', values: GroupEditorValues): void;
    (e: 'cancel'): void;
}>();

const slots = useSlots();
const { t } = useTranslations('group');
const { data: channelsData } = useChannelList();

// defaultRelayConfig 提供创建分组时的前端初始配置。
const defaultRelayConfig: GroupRelayConfig = {
    member_max_attempts: 2,
    member_retry_interval_seconds: 1,
    member_non_stream_response_timeout_seconds: 120,
    member_stream_first_event_timeout_seconds: 30,
    member_cooldown_seconds: 60,
};

const modelChannels = computed<SelectedMember[]>(() => {
    const list = channelsData.value ?? [];
    return list.flatMap(({ raw: channel }) =>
        channel.models.map((channelModel) => ({
            id: String(channelModel.id),
            channel_model_id: channelModel.id,
            name: channelModel.name,
            enabled: channel.enabled,
            channel_id: channelModel.channel_id,
            channel_name: channel.name,
        }))
    );
});

const groupName = ref(props.initial?.name ?? '');
const mode = ref<GroupMode>(props.initial?.mode ?? 'manual');
const relayConfig = ref<GroupRelayConfig>({
    ...defaultRelayConfig,
    ...props.initial?.relay_config,
});
const selectedMembers = ref<SelectedMember[]>(
    props.initial?.members ? [...props.initial.members] : []
);
const removingIds = ref<Set<string>>(new Set());
const searchKeyword = ref('');

const groupKey = computed(() => normalizeKey(groupName.value));
const selectedKeys = computed(() => new Set(selectedMembers.value.map(memberKey)));
const normalizedSearch = computed(() => searchKeyword.value.trim().toLowerCase());

const channels = computed(() => {
    const byId = new Map<number, { id: number; name: string; models: SelectedMember[] }>();
    modelChannels.value.forEach((mc) => {
        const existing = byId.get(mc.channel_id);
        if (existing) {
            existing.models.push(mc);
        } else {
            byId.set(mc.channel_id, { id: mc.channel_id, name: mc.channel_name, models: [mc] });
        }
    });

    return Array.from(byId.values())
        .map((c) => ({
            ...c,
            models: [...c.models].sort((a, b) => a.name.localeCompare(b.name)),
        }))
        .sort((a, b) => a.id - b.id);
});

const filteredChannels = computed(() => {
    const search = normalizedSearch.value;
    if (!search) return channels.value;

    return channels.value.reduce<typeof channels.value>((acc, channel) => {
        if (channel.name.toLowerCase().includes(search)) {
            acc.push(channel);
            return acc;
        }

        const models = channel.models.filter((model) =>
            model.name.toLowerCase().includes(search)
        );
        if (models.length > 0) {
            acc.push({ ...channel, models });
        }
        return acc;
    }, []);
});

const matchedModelChannels = computed(() => {
    const key = groupKey.value;
    if (!key) return [];
    return modelChannels.value.filter((mc) => matchesGroupName(mc.name, key));
});

const autoAddDisabled = computed(() => {
    const key = groupKey.value;
    if (!key || matchedModelChannels.value.length === 0) return true;
    const existing = new Set(selectedMembers.value.map((m) => m.id));
    return matchedModelChannels.value.every((mc) => existing.has(memberKey(mc)));
});

function handleAddMember(channel: SelectedMember) {
    const key = memberKey(channel);
    if (!selectedMembers.value.some((m) => m.id === key)) {
        selectedMembers.value.push({ ...channel, id: key });
    }
}

function handleAutoAdd() {
    if (matchedModelChannels.value.length === 0) return;
    const existing = new Set(selectedMembers.value.map((m) => m.id));
    const toAdd = matchedModelChannels.value
        .filter((mc) => !existing.has(memberKey(mc)))
        .map((mc) => ({ ...mc, id: memberKey(mc) }));

    if (toAdd.length > 0) {
        selectedMembers.value.push(...toAdd);
    }
}

function handleRemoveMember(id: string) {
    removingIds.value.add(id);
    removingIds.value = new Set(removingIds.value);
    window.setTimeout(() => {
        selectedMembers.value = selectedMembers.value.filter((m) => m.id !== id);
        removingIds.value.delete(id);
        removingIds.value = new Set(removingIds.value);
    }, 200);
}

function handleClearMembers() {
    selectedMembers.value = [];
    removingIds.value = new Set();
}

function handleReorderMembers(next: SelectedMember[]) {
    selectedMembers.value = [...next];
}

const isValid = computed(() => groupKey.value.length > 0 && selectedMembers.value.length > 0);

function handleSubmit() {
    if (!isValid.value || props.isSubmitting) return;
    emit('submit', {
        name: groupName.value,
        mode: mode.value,
        relay_config: { ...relayConfig.value },
        members: [...selectedMembers.value],
    });
}

function handleCancel() {
    props.onCancel?.();
    emit('cancel');
}

const hasCancel = computed(() => Boolean(props.onCancel || slots.cancel));
</script>

<template>
    <form class="flex flex-col h-full min-h-0" @submit.prevent="handleSubmit">
        <div class="flex-1 min-h-0 overflow-hidden px-1">
            <FieldGroup class="gap-4 flex flex-col min-h-0 h-full">
                <!-- 名称与路由模式 -->
                <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
                    <Field>
                        <FieldLabel for="group-name">{{ t('form.name') }}</FieldLabel>
                        <Input
                            id="group-name"
                            v-model="groupName"
                            class="rounded-xl"
                        />
                    </Field>
                    <Field>
                        <FieldLabel for="group-mode">
                            {{ t('form.mode') }}
                            <Tooltip>
                                <TooltipTrigger as-child>
                                    <LuHelpCircle class="size-4 cursor-help text-muted-foreground" />
                                </TooltipTrigger>
                                <TooltipContent side="top" :side-offset="10" align="center">
                                    {{ t('form.modeHint') }}
                                </TooltipContent>
                            </Tooltip>
                        </FieldLabel>
                        <Select v-model="mode">
                            <SelectTrigger id="group-mode" class="w-full rounded-xl">
                                <SelectValue />
                            </SelectTrigger>
                            <SelectContent>
                                <SelectItem value="manual">{{ t('form.manual') }}</SelectItem>
                                <SelectItem value="failover">{{ t('form.failover') }}</SelectItem>
                            </SelectContent>
                        </Select>
                    </Field>
                </div>

                <!-- 成员与路由策略页签 -->
                <Tabs default-value="members" class="flex flex-1 min-h-0">
                    <TabsList class="grid w-full shrink-0 grid-cols-2">
                        <TabsTrigger value="members">{{ t('form.members') }}</TabsTrigger>
                        <TabsTrigger value="relay">{{ t('form.relay') }}</TabsTrigger>
                    </TabsList>

                    <!-- 成员管理 -->
                    <TabsContent value="members" class="min-h-0 overflow-hidden">
                        <div class="grid h-full min-h-0 grid-cols-1 gap-4 md:grid-cols-2">
                            <!-- 左侧：渠道与模型选择 -->
                            <div class="rounded-xl border border-border/50 bg-muted/30 flex flex-col min-h-0">
                                <div class="grid grid-cols-[1fr_auto_1fr] items-center gap-2 px-3 py-2 border-b border-border/30 bg-muted/50">
                                    <span class="min-w-0 justify-self-start text-sm font-medium text-foreground">
                                        {{ t('form.addItem') }}
                                    </span>

                                    <div class="relative justify-self-center w-30">
                                        <LuSearch class="pointer-events-none absolute left-2 top-1/2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                                        <Input
                                            v-model="searchKeyword"
                                            class="h-6 rounded-lg border-border/60 bg-background/70 pl-7 pr-2 text-xs shadow-none focus-visible:border-border/60 focus-visible:ring-0"
                                            aria-label="search"
                                        />
                                    </div>

                                    <button
                                        type="button"
                                        :class="cn(
                                            'justify-self-end shrink-0 flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium transition-colors',
                                            autoAddDisabled
                                                ? 'text-muted-foreground/50 cursor-not-allowed'
                                                : 'hover:bg-muted text-muted-foreground hover:text-foreground'
                                        )"
                                        :disabled="autoAddDisabled"
                                        :title="t('form.autoAdd')"
                                        @click="handleAutoAdd"
                                    >
                                        <LuSparkles class="size-3.5" />
                                        <span>{{ t('form.autoAdd') }}</span>
                                    </button>
                                </div>

                                <div class="flex-1 min-h-0 overflow-y-auto p-2">
                                    <Accordion type="multiple" class="w-full space-y-2">
                                        <AccordionItem
                                            v-for="channel in filteredChannels"
                                            :key="channel.id"
                                            :value="`channel-${channel.id}`"
                                            class="border-b-0"
                                        >
                                            <div class="rounded-lg bg-muted sticky top-0 z-10 flex px-2 overflow-hidden">
                                                <AccordionTrigger class="py-3 hover:no-underline flex-1">
                                                    <span class="truncate">{{ channel.name }}</span>
                                                    <span class="text-xs text-muted-foreground shrink-0 ml-auto mr-1">
                                                        {{ channel.models.length - channel.models.reduce((acc, m) => acc + (selectedKeys.has(memberKey(m)) ? 1 : 0), 0) }}/{{ channel.models.length }}
                                                    </span>
                                                </AccordionTrigger>
                                            </div>
                                            <AccordionContent class="px-2 pt-2">
                                                <div class="flex flex-col gap-1.5">
                                                    <button
                                                        v-for="m in channel.models"
                                                        :key="memberKey(m)"
                                                        type="button"
                                                        :disabled="selectedKeys.has(memberKey(m))"
                                                        :class="cn(
                                                            'w-full flex items-center justify-between gap-2 rounded-lg border border-border/50 bg-background px-2.5 py-2 text-left transition-colors',
                                                            selectedKeys.has(memberKey(m))
                                                                ? 'opacity-60 cursor-not-allowed'
                                                                : 'hover:bg-muted'
                                                        )"
                                                        @click="!selectedKeys.has(memberKey(m)) && handleAddMember(m)"
                                                    >
                                                        <span class="flex items-center gap-2 min-w-0">
                                                            <component
                                                                :is="getModelIcon(m.name).Icon"
                                                                aria-hidden="true"
                                                                :class="getModelIcon(m.name).className"
                                                                :style="{ width: '16px', height: '16px' }"
                                                            />
                                                            <span class="text-sm font-medium truncate">{{ m.name }}</span>
                                                        </span>

                                                        <span class="shrink-0 text-muted-foreground">
                                                            <LuCheck v-if="selectedKeys.has(memberKey(m))" class="size-4 text-primary" />
                                                            <LuPlus v-else class="size-4" />
                                                        </span>
                                                    </button>
                                                </div>
                                            </AccordionContent>
                                        </AccordionItem>
                                    </Accordion>
                                </div>
                            </div>

                            <!-- 右侧：已选成员顺序 -->
                            <div class="rounded-xl border border-border/50 bg-muted/30 flex flex-col min-h-0">
                                <div class="flex items-center justify-between px-3 py-2 border-b border-border/30 bg-muted/50">
                                    <span class="text-sm font-medium text-foreground">
                                        {{ t('form.items') }}
                                        <span v-if="selectedMembers.length > 0" class="ml-1.5 text-xs text-muted-foreground font-normal">
                                            ({{ selectedMembers.length }})
                                        </span>
                                    </span>
                                    <button
                                        type="button"
                                        :disabled="selectedMembers.length === 0"
                                        :class="cn(
                                            'flex items-center gap-1 px-2 py-1 rounded-lg text-xs font-medium transition-colors',
                                            selectedMembers.length === 0
                                                ? 'text-muted-foreground/50 cursor-not-allowed'
                                                : 'hover:bg-muted text-muted-foreground hover:text-foreground'
                                        )"
                                        :title="t('form.clear')"
                                        @click="handleClearMembers"
                                    >
                                        <LuTrash2 class="size-3.5" />
                                        <span>{{ t('form.clear') }}</span>
                                    </button>
                                </div>

                                <div class="flex-1 min-h-0">
                                    <MemberList
                                        :members="selectedMembers"
                                        :removing-ids="removingIds"
                                        :show-confirm-delete="false"
                                        @reorder="handleReorderMembers"
                                        @remove="handleRemoveMember"
                                    />
                                </div>
                            </div>
                        </div>
                    </TabsContent>

                    <!-- 路由策略设置 -->
                    <TabsContent value="relay" class="min-h-0 overflow-y-auto px-1">
                        <div class="grid grid-cols-1 gap-4 md:grid-cols-2">
                            <Field>
                                <FieldLabel for="group-retry-count">
                                    {{ t('form.retryCount') }}
                                    <Tooltip>
                                        <TooltipTrigger as-child>
                                            <LuHelpCircle class="size-4 cursor-help text-muted-foreground" />
                                        </TooltipTrigger>
                                        <TooltipContent side="top" :side-offset="10" align="center">
                                            {{ t('form.retryCountHint') }}
                                        </TooltipContent>
                                    </Tooltip>
                                </FieldLabel>
                                <Input
                                    id="group-retry-count"
                                    type="number"
                                    inputmode="numeric"
                                    :min="0"
                                    :step="1"
                                    :model-value="String(relayConfig.member_max_attempts)"
                                    class="rounded-xl"
                                    @update:model-value="(val) => {
                                        const parsed = Number.parseInt(String(val), 10);
                                        relayConfig.member_max_attempts = Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
                                    }"
                                />
                            </Field>

                            <Field>
                                <FieldLabel for="group-retry-interval">
                                    {{ t('form.retryInterval') }}
                                    <Tooltip>
                                        <TooltipTrigger as-child>
                                            <LuHelpCircle class="size-4 cursor-help text-muted-foreground" />
                                        </TooltipTrigger>
                                        <TooltipContent side="top" :side-offset="10" align="center">
                                            {{ t('form.retryIntervalHint') }}
                                        </TooltipContent>
                                    </Tooltip>
                                </FieldLabel>
                                <Input
                                    id="group-retry-interval"
                                    type="number"
                                    inputmode="numeric"
                                    :min="1"
                                    :step="1"
                                    :model-value="String(relayConfig.member_retry_interval_seconds)"
                                    class="rounded-xl"
                                    @update:model-value="(val) => {
                                        const parsed = Number.parseInt(String(val), 10);
                                        relayConfig.member_retry_interval_seconds = Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
                                    }"
                                />
                            </Field>

                            <Field>
                                <FieldLabel for="group-non-stream-timeout">
                                    {{ t('form.nonStreamTimeout') }}
                                    <Tooltip>
                                        <TooltipTrigger as-child>
                                            <LuHelpCircle class="size-4 cursor-help text-muted-foreground" />
                                        </TooltipTrigger>
                                        <TooltipContent side="top" :side-offset="10" align="center">
                                            {{ t('form.nonStreamTimeoutHint') }}
                                        </TooltipContent>
                                    </Tooltip>
                                </FieldLabel>
                                <Input
                                    id="group-non-stream-timeout"
                                    type="number"
                                    inputmode="numeric"
                                    :min="1"
                                    :step="1"
                                    :model-value="String(relayConfig.member_non_stream_response_timeout_seconds)"
                                    class="rounded-xl"
                                    @update:model-value="(val) => {
                                        const parsed = Number.parseInt(String(val), 10);
                                        relayConfig.member_non_stream_response_timeout_seconds = Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
                                    }"
                                />
                            </Field>

                            <Field>
                                <FieldLabel for="group-stream-timeout">
                                    {{ t('form.streamTimeout') }}
                                    <Tooltip>
                                        <TooltipTrigger as-child>
                                            <LuHelpCircle class="size-4 cursor-help text-muted-foreground" />
                                        </TooltipTrigger>
                                        <TooltipContent side="top" :side-offset="10" align="center">
                                            {{ t('form.streamTimeoutHint') }}
                                        </TooltipContent>
                                    </Tooltip>
                                </FieldLabel>
                                <Input
                                    id="group-stream-timeout"
                                    type="number"
                                    inputmode="numeric"
                                    :min="1"
                                    :step="1"
                                    :model-value="String(relayConfig.member_stream_first_event_timeout_seconds)"
                                    class="rounded-xl"
                                    @update:model-value="(val) => {
                                        const parsed = Number.parseInt(String(val), 10);
                                        relayConfig.member_stream_first_event_timeout_seconds = Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
                                    }"
                                />
                            </Field>

                            <Field>
                                <FieldLabel for="group-cooldown">
                                    {{ t('form.cooldown') }}
                                    <Tooltip>
                                        <TooltipTrigger as-child>
                                            <LuHelpCircle class="size-4 cursor-help text-muted-foreground" />
                                        </TooltipTrigger>
                                        <TooltipContent side="top" :side-offset="10" align="center">
                                            {{ t('form.cooldownHint') }}
                                        </TooltipContent>
                                    </Tooltip>
                                </FieldLabel>
                                <Input
                                    id="group-cooldown"
                                    type="number"
                                    inputmode="numeric"
                                    :min="1"
                                    :step="1"
                                    :model-value="String(relayConfig.member_cooldown_seconds)"
                                    class="rounded-xl"
                                    @update:model-value="(val) => {
                                        const parsed = Number.parseInt(String(val), 10);
                                        relayConfig.member_cooldown_seconds = Number.isFinite(parsed) && parsed >= 1 ? parsed : 1;
                                    }"
                                />
                            </Field>
                        </div>
                    </TabsContent>
                </Tabs>
            </FieldGroup>
        </div>

        <div class="pt-4 mt-auto shrink-0">
            <div class="flex gap-2">
                <Button
                    v-if="hasCancel"
                    type="button"
                    variant="secondary"
                    class="flex-1 rounded-xl h-11"
                    @click="handleCancel"
                >
                    {{ t('detail.actions.cancel') }}
                </Button>
                <Button
                    type="submit"
                    :disabled="!isValid || isSubmitting"
                    class="flex-1 rounded-xl h-11"
                >
                    {{ isSubmitting ? submittingText : submitText }}
                </Button>
            </div>
        </div>
    </form>
</template>
