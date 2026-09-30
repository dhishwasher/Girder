function verifySolutionScenario(setup: { scenario: string }) {
    console.log(setup.scenario);
}

function getIndirectProject(id: string) {
    return { tsconfigIndirect: id, indirect: id };
}

function onlyInLost() {
    console.log("only in lost");
}

function onlyInSurvivor() {
    console.log("only in survivor");
}

describe("outer group", () => {
    describe("when default project is solution project", () => {
        it("when project is directly referenced by solution", () => {
            verifySolutionScenario({
                scenario: "project is directly referenced by solution",
            });
        });

        it("when project is indirectly referenced by solution", () => {
            onlyInLost();
            const a = getIndirectProject("1");
            const b = getIndirectProject("2");
            verifySolutionScenario({
                scenario: "project is indirectly referenced by solution",
            });
        });

        it("disables looking into the child project", () => {
            verifySolutionScenario({
                scenario: "disables looking",
            });
        });
    });

    describe("another group", () => {
        it("when project is indirectly referenced by solution", () => {
            onlyInSurvivor();
            const a = getIndirectProject("3");
            verifySolutionScenario({
                scenario: "duplicate scenario name",
            });
        });
    });
});
