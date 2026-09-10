#include "otto/cli.h"

#include <stdint.h>
#include <stdio.h>
#include <stdlib.h>
#include <string.h>

#include "otto/util.h"

static int take_option_value(
    int argc,
    char **argv,
    int *index,
    const char *argument,
    const char *option,
    char **destination
)
{
    size_t option_length;
    const char *value = NULL;

    option_length = strlen(option);
    if (strcmp(argument, option) == 0) {
        if (*index + 1 >= argc) {
            fprintf(stderr, "otto: 选项 %s 缺少值\n", option);
            return -1;
        }
        value = argv[++(*index)];
    } else if (strncmp(argument, option, option_length) == 0 &&
               argument[option_length] == '=') {
        value = argument + option_length + 1U;
    } else {
        return 0;
    }

    if (value[0] == '\0') {
        fprintf(stderr, "otto: 选项 %s 的值不能为空\n", option);
        return -1;
    }

    if (value[0] == '-' && value[1] == '-') {
        fprintf(stderr, "otto: 选项 %s 缺少值\n", option);
        return -1;
    }

    if (otto_set_string(destination, value) != 0) {
        fprintf(stderr, "otto: 内存分配失败\n");
        return -1;
    }

    return 1;
}

static int take_alias_option_value(
    int argc,
    char **argv,
    int *index,
    const char *argument,
    const char *option_one,
    const char *option_two,
    char **destination
)
{
    int result;

    result = take_option_value(
        argc,
        argv,
        index,
        argument,
        option_one,
        destination
    );
    if (result != 0) {
        return result;
    }

    return take_option_value(
        argc,
        argv,
        index,
        argument,
        option_two,
        destination
    );
}

OttoExitCode otto_cli_parse(int argc, char **argv, OttoCliOptions *options)
{
    int index;
    int result;

    if (options == NULL) {
        return OTTO_ERR_USAGE;
    }

    memset(options, 0, sizeof(*options));
    options->command = OTTO_COMMAND_ASK;
    options->prompt_arg_start = 1;

    if (argc < 2) {
        fprintf(stderr, "otto: 请提供问题，或使用 otto --help 查看帮助\n");
        return OTTO_ERR_USAGE;
    }

    if (strcmp(argv[1], "--help") == 0 || strcmp(argv[1], "-h") == 0) {
        options->command = OTTO_COMMAND_HELP;
        return OTTO_OK;
    }

    if (strcmp(argv[1], "--version") == 0 || strcmp(argv[1], "-V") == 0) {
        options->command = OTTO_COMMAND_VERSION;
        return OTTO_OK;
    }

    if (strcmp(argv[1], "--mode") == 0 ||
        strncmp(argv[1], "--mode=", 7U) == 0) {
        const char *mode_value = NULL;

        if (strncmp(argv[1], "--mode=", 7U) == 0) {
            mode_value = argv[1] + 7U;
            options->prompt_arg_start = 2;
        } else if (argc == 2) {
            options->raw_mode = 1;
            options->prompt_arg_start = 2;
            options->command = OTTO_COMMAND_MODE;
            return OTTO_OK;
        } else if (strcmp(argv[2], "--") == 0) {
            options->raw_mode = 1;
            options->prompt_arg_start = 3;
            if (argc == 3) {
                options->command = OTTO_COMMAND_MODE;
            }
            return OTTO_OK;
        } else {
            mode_value = argv[2];
            options->prompt_arg_start = 3;
        }

        if (mode_value[0] == '\0') {
            options->raw_mode = 1;
        } else if (otto_set_string(&options->mode, mode_value) != 0) {
            fprintf(stderr, "otto: 内存分配失败\n");
            return OTTO_ERR_MEMORY;
        }

        if (options->prompt_arg_start < argc &&
            strcmp(argv[options->prompt_arg_start], "--") == 0) {
            options->prompt_arg_start++;
        }
        if (options->prompt_arg_start >= argc) {
            options->command = OTTO_COMMAND_MODE;
        }
        return OTTO_OK;
    }

    if (strcmp(argv[1], "--config") == 0) {
        options->command = OTTO_COMMAND_CONFIG;

        for (index = 2; index < argc; index++) {
            if (strcmp(argv[index], "--help") == 0 ||
                strcmp(argv[index], "-h") == 0) {
                options->command = OTTO_COMMAND_HELP;
                return OTTO_OK;
            }

            result = take_option_value(
                argc, argv, &index, argv[index], "--name", &options->name
            );
            if (result != 0) {
                if (result < 0) {
                    return OTTO_ERR_USAGE;
                }
                options->has_config_options = 1;
                continue;
            }

            result = take_alias_option_value(
                argc,
                argv,
                &index,
                argv[index],
                "--baseurl",
                "--base-url",
                &options->baseurl
            );
            if (result != 0) {
                if (result < 0) {
                    return OTTO_ERR_USAGE;
                }
                options->has_config_options = 1;
                continue;
            }

            result = take_alias_option_value(
                argc,
                argv,
                &index,
                argv[index],
                "--apikey",
                "--api-key",
                &options->apikey
            );
            if (result != 0) {
                if (result < 0) {
                    return OTTO_ERR_USAGE;
                }
                options->has_config_options = 1;
                continue;
            }

            result = take_option_value(
                argc, argv, &index, argv[index], "--model", &options->model
            );
            if (result != 0) {
                if (result < 0) {
                    return OTTO_ERR_USAGE;
                }
                options->has_config_options = 1;
                continue;
            }

            fprintf(stderr, "otto: 未知配置选项：%s\n", argv[index]);
            return OTTO_ERR_USAGE;
        }

        return OTTO_OK;
    }

    if (strcmp(argv[1], "--") == 0) {
        options->prompt_arg_start = 2;
        if (options->prompt_arg_start >= argc) {
            fprintf(stderr, "otto: -- 后面缺少问题内容\n");
            return OTTO_ERR_USAGE;
        }
        return OTTO_OK;
    }

    if (argv[1][0] == '-') {
        fprintf(stderr, "otto: 未知选项：%s\n", argv[1]);
        fprintf(stderr, "otto: 如果问题以 - 开头，请使用 otto -- <问题>\n");
        return OTTO_ERR_USAGE;
    }

    return OTTO_OK;
}

char *otto_cli_join_prompt(int argc, char **argv, int start_index)
{
    size_t total_length = 0;
    size_t argument_length;
    int index;
    char *prompt;
    char *cursor;

    if (argv == NULL || start_index < 0 || start_index >= argc) {
        return NULL;
    }

    for (index = start_index; index < argc; index++) {
        argument_length = strlen(argv[index]);
        if (argument_length > SIZE_MAX - total_length) {
            return NULL;
        }
        total_length += argument_length;
        if (index + 1 < argc) {
            if (total_length == SIZE_MAX) {
                return NULL;
            }
            total_length++;
        }
    }

    if (total_length > OTTO_MAX_PROMPT_BYTES) {
        fprintf(stderr, "otto: 问题内容超过 1 MiB 限制\n");
        return NULL;
    }

    prompt = malloc(total_length + 1U);
    if (prompt == NULL) {
        return NULL;
    }

    cursor = prompt;
    for (index = start_index; index < argc; index++) {
        argument_length = strlen(argv[index]);
        memcpy(cursor, argv[index], argument_length);
        cursor += argument_length;
        if (index + 1 < argc) {
            *cursor++ = ' ';
        }
    }
    *cursor = '\0';

    return prompt;
}

void otto_cli_print_help(void)
{
    puts("OTTO (One-time.Talk once) - CLI 单轮大模型问答工具");
    puts("");
    puts("用法:");
    puts("  otto <问题内容...>");
    puts("  otto --mode <模式>             (切换并保存模式)");
    puts("  otto --mode                    (清除模式，恢复裸生成)");
    puts("  otto --mode <模式> <问题内容...>  (单次使用模式)");
    puts("  otto --mode -- <问题内容...>     (单次裸生成)");
    puts("  otto --config");
    puts("  otto --config --name Openai --baseurl <URL> --apikey <KEY> [--model <MODEL>]");
    puts("  otto --help");
    puts("  otto --version");
    puts("");
    puts("示例:");
    puts("  otto 你好");
    puts("  otto 请用一句话解释什么是 TCP");
    puts("  otto --mode otto");
    puts("  otto --mode test 你好");
    puts("  otto --mode");
    puts("  otto --mode -- 你好");
    puts("  otto -- 北京 今天 天气 怎么样");
    puts("");
    puts("说明:");
    puts("  问题参数会自动用空格拼接，因此通常不需要加引号。");
    puts("  推荐使用 otto --config 进入交互式配置，API Key 不会显示在屏幕上。");
    puts("  普通 otto <问题> 会使用已保存模式；未选择模式时不添加 system prompt。");
    puts("  name 只是服务名称；只要服务兼容 OpenAI Chat Completions 即可使用。");
    puts("  配置文件默认位于 $XDG_CONFIG_HOME/otto/config 或 ~/.config/otto/config。");
    puts("  可使用 OTTO_CONFIG 环境变量覆盖配置文件路径。");
}

void otto_cli_print_version(void)
{
    puts("otto " OTTO_VERSION);
}

void otto_cli_free(OttoCliOptions *options)
{
    if (options == NULL) {
        return;
    }

    free(options->name);
    free(options->baseurl);
    free(options->apikey);
    free(options->model);
    free(options->mode);
    memset(options, 0, sizeof(*options));
}
