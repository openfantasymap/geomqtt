// Copyright openfantasymap and geomqtt contributors. Dual MIT / Apache-2.0.

using UnrealBuildTool;

public class Geomqtt : ModuleRules
{
    public Geomqtt(ReadOnlyTargetRules Target) : base(Target)
    {
        PCHUsage = ModuleRules.PCHUsageMode.UseExplicitOrSharedPCHs;
        bEnableExceptions = false;

        PublicDependencyModuleNames.AddRange(new string[]
        {
            "Core",
            "CoreUObject",
            "Engine"
        });

        PrivateDependencyModuleNames.AddRange(new string[]
        {
            "WebSockets",
            "Json",
            "JsonUtilities"
        });
    }
}
