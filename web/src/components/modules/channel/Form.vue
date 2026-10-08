<script setup lang="ts">
import { ref, reactive, computed } from 'vue';
import {
    ChannelType,
    type Channel,
    type ChannelModelInput,
    type UpdateChannelRequest,
    type CustomHeader,
    useCreateChannel,
    useFetchModel,
    useUpdateChannel,
} from '@/api/channel';
import { useMorphingDialog } from '@/components/ui/morphing-dialog';
import {
    Select,
    SelectContent,
    SelectItem,
    SelectTrigger,
    SelectValue,
} from '@/components/ui/select';
import { Switch } from '@/components/ui/switch';
import { Button } from '@/components/ui/button';
import { Input } from '@/components/ui/input';
import { Badge } from '@/components/ui/badge';
import { Checkbox } from '@/components/ui/checkbox';
import {
    Accordion,
    AccordionContent,
    AccordionItem,
    AccordionTrigger,
} from '@/components/ui/accordion';
import { toast } from '@/composables/toast';
import { useTranslations } from '@/composables/i18n';
import { LuRefreshCw, LuX, LuPlus } from 'vue-icons-plus/lu';

interface ChannelFormData {
    name: string;
    type: ChannelType;
    base_url: string;
    key: string;
    custom_header: CustomHeader[];
    passthrough_header: string[];
    param_override: string;
    models: ChannelModelInput[];
    enabled: boolean;
    auto_sync: boolean;
}

const props = defineProps<{
    channel?: Channel;
}>();

const { t } = useTranslations('channel.form');
const { setIsOpen } = useMorphingDialog();
const createChannel = useCreateChannel();
const updateChannel = useUpdateChannel();

const emptyFormData: ChannelFormData = {
    name: '',
    type: ChannelType.OpenaiCompletion,
    base_url: '',
    key: '',
    custom_header: [{ header_key: '', header_value: '' }],
    passthrough_header: [],
    param_override: '',
    models: [],
    auto_sync: false,
    enabled: true,
};

const formData = reactive<ChannelFormData>(
    props.channel
        ? {
              name: props.channel.name,
              type: props.channel.type,
              enabled: props.channel.enabled,
              base_url: props.channel.base_url,
              key: props.channel.key,
              custom_header:
                  props.channel.custom_header.length > 0
                      ? props.channel.custom_header.map((h) => ({ ...h }))
                      : [{ header_key: '', header_value: '' }],
              param_override: props.channel.param_override ?? '',
              passthrough_header: [...(props.channel.passthrough_header ?? [])],
              models: props.channel.models.map(({ name, source }) => ({ name, source })),
              auto_sync: props.channel.auto_sync,
          }
        : {
              ...emptyFormData,
              custom_header: [{ header_key: '', header_value: '' }],
              models: [],
          }
);

// 新建和编辑共存时隔离表单控件的 id
const idPrefix = computed(() => (props.channel ? `channel-${props.channel.id}` : 'new-channel'));
const isPending = computed(() => (props.channel ? updateChannel.isPending.value : createChannel.isPending.value));
const autoModels = computed(() => formData.models.filter((model) => model.source === 'auto').map((model) => model.name));
const manualModels = computed(() => formData.models.filter((model) => model.source === 'manual').map((model) => model.name));
const hasModels = computed(() => formData.models.length > 0);

// 透传名单规范化：去两侧空白、丢空项、统一小写并去重（Header 名大小写不敏感）
const normalizedPassthrough = computed(() =>
    Array.from(new Set(formData.passthrough_header.map((name) => name.trim().toLowerCase()).filter(Boolean)))
);

const inputValue = ref('');
// 刷新获取到的候选模型列表（不会自动选中，由用户勾选）
const availableModels = ref<string[]>([]);
// 候选模型搜索词：仅过滤展示，不影响全选/取消全选的作用范围
const modelSearch = ref('');
const filteredAvailableModels = computed(() => {
    const keyword = modelSearch.value.trim().toLowerCase();
    if (!keyword) return availableModels.value;
    return availableModels.value.filter((name) => name.toLowerCase().includes(keyword));
});
const trimmedInput = computed(() => inputValue.value.trim());
// 输入的模型名非空且未添加过时才允许追加
const canAddModel = computed(
    () =>
        trimmedInput.value !== '' &&
        !manualModels.value.includes(trimmedInput.value) &&
        !autoModels.value.includes(trimmedInput.value)
);

const fetchModel = useFetchModel();

// 忽略顺序生成模型集合的比较键
const modelsKey = (models: ChannelModelInput[]) =>
    models
        .map((model) => `${model.source}:${model.name}`)
        .sort()
        .join(',');

const updateModels = (nextAuto: string[], nextManual: string[]) => {
    const models: ChannelModelInput[] = [
        ...nextAuto.map((name) => ({ name, source: 'auto' as const })),
        ...nextManual.map((name) => ({ name, source: 'manual' as const })),
    ];
    if (JSON.stringify(formData.models) === JSON.stringify(models)) return;
    formData.models = models;
};

const handleRefreshModels = () => {
    if (!formData.base_url || !formData.key) return;
    fetchModel.mutate(
        {
            type: formData.type,
            base_url: formData.base_url.trim(),
            key: formData.key.trim(),
            custom_header: formData.custom_header.filter((h) => h.header_key.trim()),
        },
        {
            onSuccess: (data) => {
                if (data && data.length > 0) {
                    // 只填充候选列表，不自动选中，由用户决定勾选哪些模型
                    availableModels.value = Array.from(new Set(data.map((m) => m.trim()).filter(Boolean)));
                    modelSearch.value = '';
                    toast.success(t('modelRefreshSuccess'));
                } else {
                    toast.warning(t('modelRefreshEmpty'));
                }
            },
            onError: (error) => {
                const errorMessage = error instanceof Error ? error.message : String(error);
                toast.error(`${t('modelRefreshFailed')}: ${errorMessage}`);
            },
        }
    );
};

const handleAddModel = () => {
    if (canAddModel.value) updateModels(autoModels.value, [...manualModels.value, trimmedInput.value]);
    inputValue.value = '';
};

const handleInputKeyDown = (e: KeyboardEvent) => {
    if (e.key === 'Enter') {
        e.preventDefault();
        if (trimmedInput.value) handleAddModel();
    }
};

const handleUpdateHeader = (idx: number, patch: Partial<CustomHeader>) => {
    formData.custom_header = formData.custom_header.map((h, i) => (i === idx ? { ...h, ...patch } : h));
};

// 新建提交全部字段; 编辑只提交变化字段, 空串对应后端的清空语义
const handleSubmit = (event: Event) => {
    event.preventDefault();
    if (!hasModels.value) return;

    const custom_header = formData.custom_header
        .map((h) => ({ header_key: h.header_key.trim(), header_value: h.header_value }))
        .filter((h) => h.header_key && h.header_value !== '');

    if (!props.channel) {
        createChannel.mutate(
            {
                name: formData.name,
                type: formData.type,
                enabled: formData.enabled,
                base_url: formData.base_url.trim(),
                key: formData.key.trim(),
                models: formData.models,
                auto_sync: formData.auto_sync,
                custom_header,
                passthrough_header: normalizedPassthrough.value,
                param_override: formData.param_override.trim(),
            },
            {
                onSuccess: () => setIsOpen(false),
            }
        );
        return;
    }

    const req: UpdateChannelRequest = { id: props.channel.id };
    if (formData.name !== props.channel.name) req.name = formData.name;
    if (formData.type !== props.channel.type) req.type = formData.type;
    if (formData.enabled !== props.channel.enabled) req.enabled = formData.enabled;
    if (formData.base_url.trim() !== props.channel.base_url) req.base_url = formData.base_url.trim();
    if (formData.key.trim() !== props.channel.key) req.key = formData.key.trim();
    if (modelsKey(formData.models) !== modelsKey(props.channel.models)) req.models = formData.models;
    if (formData.auto_sync !== props.channel.auto_sync) req.auto_sync = formData.auto_sync;
    if (JSON.stringify(custom_header) !== JSON.stringify(props.channel.custom_header)) req.custom_header = custom_header;
    {
        const next = formData.param_override.trim();
        if (next !== (props.channel.param_override ?? '')) req.param_override = next;
    }
    if (JSON.stringify(normalizedPassthrough.value) !== JSON.stringify(props.channel.passthrough_header ?? [])) {
        req.passthrough_header = normalizedPassthrough.value;
    }

    updateChannel.mutate(req, {
        onSuccess: () => setIsOpen(false),
    });
};
</script>

<template>
    <form @submit="handleSubmit" class="space-y-4 px-1">
        <div class="grid grid-cols-1 md:grid-cols-2 gap-4">
            <div class="space-y-2">
                <label :for="`${idPrefix}-name`" class="text-sm font-medium text-card-foreground">
                    {{ t('name') }}
                </label>
                <Input
                    class="rounded-xl"
                    :id="`${idPrefix}-name`"
                    type="text"
                    v-model="formData.name"
                    required
                />
            </div>

            <div class="space-y-2">
                <label :for="`${idPrefix}-type`" class="text-sm font-medium text-card-foreground">
                    {{ t('type') }}
                </label>
                <Select
                    :model-value="String(formData.type)"
                    @update:model-value="(val) => (formData.type = val as ChannelType)"
                >
                    <SelectTrigger
                        :id="`${idPrefix}-type`"
                        class="rounded-xl w-full border border-border px-4 py-2 text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                    >
                        <SelectValue />
                    </SelectTrigger>
                    <SelectContent class="rounded-xl">
                        <SelectItem class="rounded-xl" :value="String(ChannelType.OpenaiCompletion)">
                            {{ t('typeOpenaiCompletion') }}
                        </SelectItem>
                        <SelectItem class="rounded-xl" :value="String(ChannelType.OpenAIResponse)">
                            {{ t('typeOpenAIResponse') }}
                        </SelectItem>
                        <SelectItem class="rounded-xl" :value="String(ChannelType.Anthropic)">
                            {{ t('typeAnthropic') }}
                        </SelectItem>
                        <SelectItem class="rounded-xl" :value="String(ChannelType.Gemini)">
                            {{ t('typeGemini') }}
                        </SelectItem>
                    </SelectContent>
                </Select>
            </div>
        </div>

        <div class="space-y-2">
            <label :for="`${idPrefix}-base-url`" class="text-sm font-medium text-card-foreground">
                {{ t('baseUrl') }}
            </label>
            <Input
                :id="`${idPrefix}-base-url`"
                type="url"
                v-model="formData.base_url"
                :placeholder="t('baseUrlUrl')"
                required
                class="rounded-xl"
            />
        </div>

        <div class="space-y-2">
            <label :for="`${idPrefix}-key`" class="text-sm font-medium text-card-foreground">
                {{ t('apiKey') }}
            </label>
            <textarea
                :id="`${idPrefix}-key`"
                v-model="formData.key"
                :placeholder="t('apiKeyPlaceholder')"
                required
                class="min-h-16 w-full rounded-xl border border-border bg-background px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
            />
        </div>

        <div class="space-y-2">
            <div class="flex items-center justify-between">
                <label class="text-sm font-medium text-card-foreground">{{ t('model') }}</label>
                <Button
                    type="button"
                    variant="ghost"
                    size="sm"
                    @click="handleRefreshModels"
                    :disabled="!formData.base_url || !formData.key || fetchModel.isPending.value"
                    class="h-6 px-2 text-xs text-muted-foreground/50 hover:text-muted-foreground hover:bg-transparent"
                >
                    <LuRefreshCw :class="['h-3 w-3 mr-1', fetchModel.isPending.value ? 'animate-spin' : '']" />
                    {{ t('modelRefresh') }}
                </Button>
            </div>
            <div class="relative">
                <Input
                    :id="`${idPrefix}-model-custom`"
                    type="text"
                    v-model="inputValue"
                    @keydown="handleInputKeyDown"
                    :placeholder="t('modelCustomPlaceholder')"
                    class="pr-10 rounded-xl"
                />
                <Button
                    v-if="canAddModel"
                    type="button"
                    variant="ghost"
                    size="sm"
                    @click="handleAddModel"
                    class="absolute rounded-lg right-1 top-1/2 -translate-y-1/2 h-7 w-7 p-0 text-muted-foreground hover:bg-accent hover:text-accent-foreground transition-colors"
                    :title="t('modelAdd')"
                >
                    <LuPlus class="size-4" />
                </Button>
            </div>
            <div v-if="availableModels.length > 0" class="space-y-2">
                <div class="flex items-center justify-between">
                    <label class="text-xs font-medium text-card-foreground">
                        {{ t('modelAvailable') }}
                        ({{ modelSearch.trim() ? `${filteredAvailableModels.length}/${availableModels.length}` : availableModels.length }})
                    </label>
                    <div class="flex items-center gap-1">
                        <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            @click="updateModels(Array.from(new Set([...autoModels, ...availableModels])), manualModels)"
                            class="h-6 px-2 text-xs text-muted-foreground/50 hover:text-muted-foreground hover:bg-transparent"
                        >
                            {{ t('modelSelectAll') }}
                        </Button>
                        <Button
                            type="button"
                            variant="ghost"
                            size="sm"
                            @click="updateModels(autoModels.filter((m) => !availableModels.includes(m)), manualModels)"
                            class="h-6 px-2 text-xs text-muted-foreground/50 hover:text-muted-foreground hover:bg-transparent"
                        >
                            {{ t('modelDeselectAll') }}
                        </Button>
                    </div>
                </div>
                <Input
                    type="text"
                    v-model="modelSearch"
                    :placeholder="t('modelSearchPlaceholder')"
                    class="rounded-xl h-8 text-sm"
                />
                <div class="rounded-xl border border-border bg-muted/30 p-2.5 max-h-40 overflow-y-auto">
                    <p v-if="filteredAvailableModels.length === 0" class="py-1 px-1 text-xs text-muted-foreground">
                        {{ t('modelSearchEmpty') }}
                    </p>
                    <label
                        v-for="name in filteredAvailableModels"
                        :key="name"
                        class="flex items-center gap-2 py-1 text-sm text-card-foreground cursor-pointer hover:bg-muted/60 rounded-lg px-1"
                    >
                        <Checkbox
                            :checked="autoModels.includes(name)"
                            @update:checked="
                                (checked) => {
                                    if (checked) {
                                        if (!autoModels.includes(name)) updateModels([...autoModels, name], manualModels);
                                    } else {
                                        updateModels(autoModels.filter((m) => m !== name), manualModels);
                                    }
                                }
                            "
                        />
                        <span class="truncate">{{ name }}</span>
                    </label>
                </div>
            </div>

            <div class="space-y-2">
                <div class="flex items-center justify-between">
                    <label class="text-xs font-medium text-card-foreground">
                        {{ t('modelSelected') }} {{ hasModels ? `(${formData.models.length})` : '' }}
                    </label>
                    <Button
                        v-if="hasModels"
                        type="button"
                        variant="ghost"
                        size="sm"
                        @click="updateModels([], [])"
                        class="h-6 px-2 text-xs text-muted-foreground/50 hover:text-muted-foreground hover:bg-transparent"
                    >
                        {{ t('modelClearAll') }}
                    </Button>
                </div>
                <div class="rounded-xl border border-border bg-muted/30 p-2.5 max-h-40 min-h-12 overflow-y-auto">
                    <div v-if="hasModels" class="flex flex-wrap gap-1.5">
                        <Badge v-for="model in autoModels" :key="model" variant="secondary" class="bg-muted hover:bg-muted/80">
                            {{ model }}
                            <button
                                type="button"
                                @click="updateModels(autoModels.filter((m) => m !== model), manualModels)"
                                class="ml-1 rounded-sm opacity-70 hover:opacity-100 focus:outline-none focus:ring-1 focus:ring-ring"
                            >
                                <LuX class="h-3 w-3" />
                            </button>
                        </Badge>
                        <Badge v-for="model in manualModels" :key="model" class="bg-primary hover:bg-primary/90">
                            {{ model }}
                            <button
                                type="button"
                                @click="updateModels(autoModels, manualModels.filter((m) => m !== model))"
                                class="ml-1 rounded-sm opacity-70 hover:opacity-100 focus:outline-none focus:ring-1 focus:ring-ring"
                            >
                                <LuX class="h-3 w-3" />
                            </button>
                        </Badge>
                    </div>
                    <div v-else class="flex items-center justify-center h-8 text-xs text-muted-foreground">
                        {{ t('modelNoSelected') }}
                    </div>
                </div>
            </div>
        </div>

        <Accordion type="single" :collapsible="true" class="w-full border rounded-xl bg-card">
            <AccordionItem value="advanced" class="border-none">
                <AccordionTrigger class="text-sm font-medium text-card-foreground py-3 px-4 hover:no-underline hover:bg-muted/30 rounded-xl transition-colors">
                    {{ t('advanced') }}
                </AccordionTrigger>
                <AccordionContent class="pt-4 px-4 pb-4 space-y-4 border-t">
                    <div class="space-y-2">
                        <div class="flex items-center justify-between">
                            <label class="text-sm font-medium text-card-foreground">
                                {{ t('customHeader') }} {{ formData.custom_header.length > 0 ? `(${formData.custom_header.length})` : '' }}
                            </label>
                            <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                @click="formData.custom_header = [...formData.custom_header, { header_key: '', header_value: '' }]"
                                class="h-6 px-2 text-xs text-muted-foreground/70 hover:text-muted-foreground hover:bg-transparent"
                            >
                                <LuPlus class="h-3 w-3 mr-1" />
                                {{ t('customHeaderAdd') }}
                            </Button>
                        </div>
                        <div class="space-y-2">
                            <div v-for="(h, idx) in formData.custom_header" :key="`hdr-${idx}`" class="flex items-center gap-2">
                                <Input
                                    type="text"
                                    v-model="h.header_key"
                                    :placeholder="t('customHeaderKey')"
                                    class="rounded-xl flex-1"
                                />
                                <Input
                                    type="text"
                                    v-model="h.header_value"
                                    :placeholder="t('customHeaderValue')"
                                    class="rounded-xl flex-1"
                                />
                                <Button
                                    type="button"
                                    variant="ghost"
                                    size="sm"
                                    @click="formData.custom_header = formData.custom_header.filter((_, i) => i !== idx)"
                                    :disabled="formData.custom_header.length <= 1"
                                    class="h-8 w-8 p-0 rounded-xl text-muted-foreground hover:text-destructive hover:bg-transparent disabled:opacity-40"
                                    title="Remove"
                                >
                                    <LuX class="h-4 w-4" />
                                </Button>
                            </div>
                        </div>
                    </div>

                    <div class="space-y-2">
                        <div class="flex items-center justify-between">
                            <label class="text-sm font-medium text-card-foreground">
                                {{ t('passthroughHeader') }} {{ formData.passthrough_header.length > 0 ? `(${formData.passthrough_header.length})` : '' }}
                            </label>
                            <Button
                                type="button"
                                variant="ghost"
                                size="sm"
                                @click="formData.passthrough_header = [...formData.passthrough_header, '']"
                                class="h-6 px-2 text-xs text-muted-foreground/70 hover:text-muted-foreground hover:bg-transparent"
                            >
                                <LuPlus class="h-3 w-3 mr-1" />
                                {{ t('passthroughHeaderAdd') }}
                            </Button>
                        </div>
                        <div class="space-y-2">
                            <div v-for="(_, idx) in formData.passthrough_header" :key="`pt-${idx}`" class="flex items-center gap-2">
                                <Input
                                    type="text"
                                    v-model="formData.passthrough_header[idx]"
                                    :placeholder="t('passthroughHeaderPlaceholder')"
                                    class="rounded-xl flex-1"
                                />
                                <Button
                                    type="button"
                                    variant="ghost"
                                    size="sm"
                                    @click="formData.passthrough_header = formData.passthrough_header.filter((_, i) => i !== idx)"
                                    class="h-8 w-8 p-0 rounded-xl text-muted-foreground hover:text-destructive hover:bg-transparent"
                                    title="Remove"
                                >
                                    <LuX class="h-4 w-4" />
                                </Button>
                            </div>
                        </div>
                        <p class="text-xs text-muted-foreground">{{ t('passthroughHeaderHint') }}</p>
                    </div>

                    <div class="space-y-2">
                        <label :for="`${idPrefix}-param-override`" class="text-sm font-medium text-card-foreground">
                            {{ t('paramOverride') }}
                        </label>
                        <textarea
                            :id="`${idPrefix}-param-override`"
                            v-model="formData.param_override"
                            :placeholder="t('paramOverridePlaceholder')"
                            class="min-h-28 w-full rounded-xl border border-border bg-background px-3 py-2 text-sm text-foreground focus-visible:outline-none focus-visible:ring-2 focus-visible:ring-ring"
                        />
                    </div>
                </AccordionContent>
            </AccordionItem>
        </Accordion>

        <div class="flex flex-wrap items-center justify-between gap-4 p-4 rounded-xl bg-muted/20 border border-border/50">
            <label class="flex items-center gap-2 cursor-pointer">
                <Switch v-model="formData.enabled" />
                <span class="text-sm font-medium text-card-foreground">{{ t('enabled') }}</span>
            </label>
            <div class="flex items-center gap-6">
                <label class="flex items-center gap-2 cursor-pointer">
                    <Switch v-model="formData.auto_sync" />
                    <span class="text-sm text-card-foreground">{{ t('autoSync') }}</span>
                </label>
            </div>
        </div>

        <div :class="['flex flex-col gap-3 pt-2', channel ? 'sm:flex-row' : '']">
            <Button
                v-if="channel"
                type="button"
                variant="secondary"
                @click="setIsOpen(false)"
                class="w-full sm:flex-1 rounded-2xl h-12"
            >
                {{ t('cancel') }}
            </Button>
            <Button
                type="submit"
                :disabled="isPending || !hasModels"
                class="w-full sm:flex-1 rounded-2xl h-12"
            >
                {{
                    channel
                        ? isPending
                            ? t('saving')
                            : t('save')
                        : isPending
                          ? t('submitting')
                          : t('submit')
                }}
            </Button>
        </div>
    </form>
</template>
