{ lib, config, ... }: {
  options = with lib; let
    # adapted from https://gist.github.com/GrafBlutwurst/2d6156321d6b89cb21a1d0702f5d853e
    addTag = typeTag: module: let
      tagModule = {
        type = mkOption {
          type = types.enum [typeTag];
          description = ''Type Tag (${typeTag})'';
        };
      };
      imports =
        if module ? imports
        then {inherit (module) imports;}
        else {};
      config =
        if module ? config
        then {inherit (module) config;}
        else {};
    in (
      assert (builtins.isAttrs module) || abort "Module passed to oneOfTagged must be a Record";
      assert (builtins.isString typeTag) || abort "TypeTag passed to oneOfTagged must be a String";
      assert !(module ? option.type) || abort "Module passed to oneOfTagged canno't have an option called type (reserved for typetag)";
        imports
        // config
        // {
          options = module.options // tagModule;
        }
    );

    taggedSubmodule = typeTag: module: let
      taggedModule = addTag typeTag module;
      baseSubmodule = types.submodule taggedModule;
      check = v: (baseSubmodule.check v) && (v.type == typeTag);
      description = "Submodule[${typeTag}]";
    in
      baseSubmodule
      // {
        inherit check;
        inherit description;
      };

    mapAttrDefs = definitions: attrValues (mapAttrs taggedSubmodule definitions);

    oneOfTagged = definitions:
      types.oneOf (mapAttrDefs definitions);

    retryPolicy = types.submodule {
      options = {
        max_retries = mkOption {
          type = types.nullOr types.int;
          default = null;
        };
        initial = mkOption {
          type = types.nullOr types.float;
          default = null;
        };
        multiplier = mkOption {
          type = types.nullOr types.float;
          default = null;
        };
      };
    };
    checkDefinitionCommon = {
      retry_policy = mkOption {
        type = types.nullOr retryPolicy;
        default = null;
      };
      check_timeout = mkOption {
        type = types.nullOr types.float;
        default = null;
      };
      labels = mkOption {
        type = types.nullOr types.attrs;
        default = null;
      };
    };

    checkDefinition = oneOfTagged {
      ssh = {
        options =
          {
            params = {
              hostname = mkOption {
                type = types.str;
                default = config.networking.hostName;
              };
              command = mkOption {
                type = types.str;
              };
              username = mkOption {
                type = types.nullOr types.str;
                default = null;
              };
            };
          }
          // checkDefinitionCommon;
      };
      http = {
        options =
          {
            params = {
              url = mkOption {
                type = types.str;
              };
            };
          }
          // checkDefinitionCommon;
      };
      dns = {
        options =
          {
            params = {
              domain = mkOption {
                type = types.str;
              };
            };
          }
          // checkDefinitionCommon;
      };
    };
  in {
    deployment.healthchecks = lib.mkOption {
      type = with lib; types.listOf checkDefinition;
      default = [];
    };
  };
}
