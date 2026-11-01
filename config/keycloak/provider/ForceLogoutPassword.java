package org.zerobudget.keycloak;

import org.keycloak.authentication.RequiredActionContext;
import org.keycloak.authentication.RequiredActionProvider;
import org.keycloak.authentication.requiredactions.UpdatePassword;
import org.keycloak.models.KeycloakSession;

/** Password recovery must revoke other devices even when the checkbox is omitted. */
public final class ForceLogoutPassword extends UpdatePassword {
    @Override
    public int order() { return 100; }

    @Override
    public RequiredActionProvider create(KeycloakSession session) {
        return new ForceLogoutPassword();
    }

    @Override
    public void processAction(RequiredActionContext context) {
        context.getHttpRequest().getDecodedFormParameters().putSingle("logout-sessions", "on");
        super.processAction(context);
    }
}
