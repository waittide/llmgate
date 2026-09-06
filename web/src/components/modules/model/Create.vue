<script setup lang="ts">
import { ref } from 'vue';
import { useCreateModel } from '@/api/model';
import { Input } from '@/components/ui/input';
import { Button } from '@/components/ui/button';
import { Field, FieldLabel, FieldGroup } from '@/components/ui/field';
import {
    MorphingDialogClose,
    MorphingDialogTitle,
    MorphingDialogDescription,
    useMorphingDialog,
} from '@/components/ui/morphing-dialog';
import { useTranslations } from '@/composables/i18n';
import { toast } from '@/composables/toast';

const { setIsOpen } = useMorphingDialog();
const { t } = useTranslations('model');
const createModel = useCreateModel();

const formData = ref({
    name: '',
    input: '',
    output: '',
    cache_read: '',
    cache_write: '',
});

const handleSubmit = () => {
    const trimmedName = formData.value.name.trim();
    if (!trimmedName) {
        toast.error(`${t('toast.createFailed')}: ${t('create.name')}`);
        return;
    }

    const parsePositiveNumber = (val: string) => {
        if (!val) return 0;
        const num = parseFloat(val);
        return Number.isFinite(num) && num >= 0 ? num : 0;
    };

    createModel.mutate(
        {
            name: trimmedName,
            input: parsePositiveNumber(formData.value.input),
            output: parsePositiveNumber(formData.value.output),
            cache_read: parsePositiveNumber(formData.value.cache_read),
            cache_write: parsePositiveNumber(formData.value.cache_write),
        },
        {
            onSuccess: () => {
                toast.success(t('toast.created'));
                formData.value = { name: '', input: '', output: '', cache_read: '', cache_write: '' };
                setIsOpen(false);
            },
            onError: (error) => {
                const msg = error instanceof Error ? error.message : String(error);
                toast.error(msg || t('toast.createFailed'));
            },
        }
    );
};
</script>

<template>
    <div class="w-screen max-w-full md:max-w-xl">
        <MorphingDialogTitle>
            <header class="mb-5 flex items-center justify-between">
                <h2 class="text-2xl font-bold text-card-foreground">{{ t('create.title') }}</h2>
                <MorphingDialogClose
                    class="relative right-0 top-0"
                    :variants="{
                        initial: { opacity: 0, scale: 0.8 },
                        animate: { opacity: 1, scale: 1 },
                        exit: { opacity: 0, scale: 0.8 },
                    }"
                />
            </header>
        </MorphingDialogTitle>
        <MorphingDialogDescription>
            <form @submit.prevent="handleSubmit">
                <FieldGroup class="gap-4">
                    <Field>
                        <FieldLabel for="model-name">{{ t('create.name') }}</FieldLabel>
                        <Input
                            id="model-name"
                            v-model="formData.name"
                            class="rounded-xl"
                        />
                    </Field>
                    <div class="grid grid-cols-2 gap-4">
                        <Field>
                            <FieldLabel for="model-input">{{ t('create.input') }}</FieldLabel>
                            <Input
                                id="model-input"
                                type="number"
                                step="any"
                                v-model="formData.input"
                                class="rounded-xl"
                            />
                        </Field>
                        <Field>
                            <FieldLabel for="model-output">{{ t('create.output') }}</FieldLabel>
                            <Input
                                id="model-output"
                                type="number"
                                step="any"
                                v-model="formData.output"
                                class="rounded-xl"
                            />
                        </Field>
                        <Field>
                            <FieldLabel for="model-cache-read">{{ t('create.cacheRead') }}</FieldLabel>
                            <Input
                                id="model-cache-read"
                                type="number"
                                step="any"
                                v-model="formData.cache_read"
                                class="rounded-xl"
                            />
                        </Field>
                        <Field>
                            <FieldLabel for="model-cache-write">{{ t('create.cacheWrite') }}</FieldLabel>
                            <Input
                                id="model-cache-write"
                                type="number"
                                step="any"
                                v-model="formData.cache_write"
                                class="rounded-xl"
                            />
                        </Field>
                    </div>
                    <Button
                        type="submit"
                        :disabled="createModel.isPending.value || !formData.name.trim()"
                        class="w-full rounded-xl h-11"
                    >
                        {{ createModel.isPending.value ? t('create.submitting') : t('create.submit') }}
                    </Button>
                </FieldGroup>
            </form>
        </MorphingDialogDescription>
    </div>
</template>
