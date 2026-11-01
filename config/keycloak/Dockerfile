FROM quay.io/keycloak/keycloak:26.8.1@sha256:3f3c5f8e03c614774b4bd9173b88391fefa4153bfc010d8012d59ad825377332 AS keycloak
FROM eclipse-temurin:21-jdk@sha256:ebfbb18d11ff163ee83b21258acf12248af48d22a0fdfdc68fa1a1516c05976a AS build
COPY --from=keycloak /opt/keycloak/lib/lib/main/ /deps/
COPY provider/ /source/
RUN mkdir -p /classes && javac --release 21 -cp '/deps/*' -d /classes /source/ForceLogoutPassword.java && cp -r /source/META-INF /classes/ && jar --create --file /force-logout-password.jar -C /classes .
FROM keycloak
COPY --from=build --chown=keycloak:keycloak /force-logout-password.jar /opt/keycloak/providers/force-logout-password.jar
ENV KC_HEALTH_ENABLED=true
