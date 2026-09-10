#ifndef OTTO_CLI_H
#define OTTO_CLI_H

#include "otto/common.h"

typedef enum {
    OTTO_COMMAND_ASK,
    OTTO_COMMAND_CONFIG,
    OTTO_COMMAND_MODE,
    OTTO_COMMAND_HELP,
    OTTO_COMMAND_VERSION
} OttoCommand;

typedef struct {
    OttoCommand command;
    int prompt_arg_start;
    int raw_mode;

    char *mode;
    char *name;
    char *baseurl;
    char *apikey;
    char *model;
    int has_config_options;
} OttoCliOptions;

OttoExitCode otto_cli_parse(int argc, char **argv, OttoCliOptions *options);
char *otto_cli_join_prompt(int argc, char **argv, int start_index);
void otto_cli_print_help(void);
void otto_cli_print_version(void);
void otto_cli_free(OttoCliOptions *options);

#endif
