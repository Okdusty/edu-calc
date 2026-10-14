% Model research: fit a damped oscillator to noisy measurements, simulate predator-prey dynamics.
rand('seed', 7); randn('seed', 7);

% "measured" displacement of a damped spring: x(t) = A e^{-k t} cos(w t + p) + noise
t = linspace(0, 10, 80);
x = 2.0*exp(-0.35*t).*cos(2.6*t + 0.3) + 0.08*randn(size(t));

% least-squares fit of the four parameters with fminsearch
model = @(q, t) q(1)*exp(-q(2)*t).*cos(q(3)*t + q(4));
cost = @(q) sum((model(q, t) - x).^2);
q = fminsearch(cost, [1.5 0.2 2.5 0]);
printf('fit: A = %.3f  k = %.3f  w = %.3f  p = %.3f\n', q);

subplot(2, 2, 1)
tt = linspace(0, 10, 600);
plot(t, x, 'o', tt, model(q, tt), tt, q(1)*exp(-q(2)*tt), '--', tt, -q(1)*exp(-q(2)*tt), '--')
grid on; legend('data', 'fit', 'envelope')
title(sprintf('x(t) = %.2f e^{-%.2f t} cos(%.2f t + %.2f)', q)); xlabel('t [s]'); ylabel('x [cm]')

subplot(2, 2, 2)
stem(t, x - model(q, t)); grid on
title(sprintf('residuals, RMS = %.3f', sqrt(mean((x - model(q, t)).^2)))); xlabel('t [s]')

% Lotka-Volterra: prey u, predators v
a = 1.1; b = 0.4; c = 0.4; d = 0.1;
f = @(t, y) [a*y(1) - b*y(1)*y(2); d*y(1)*y(2) - c*y(2)];
[T, Y] = ode45(f, [0 40], [10; 5]);

subplot(2, 2, 3)
plot(T, Y(:, 1), T, Y(:, 2)); grid on
legend('prey', 'predators'); title('Lotka-Volterra, ode45'); xlabel('t'); ylabel('population')

subplot(2, 2, 4)
[U, V] = meshgrid(linspace(0.5, 30, 16), linspace(0.5, 9, 16));
dU = a*U - b*U.*V;  dV = d*U.*V - c*V;
ux = dU/30;  uy = dV/9;  m = sqrt(ux.^2 + uy.^2);   % direction as it looks on screen
quiver(U, V, 1.5*ux./m, 0.45*uy./m); hold on
plot(Y(:, 1), Y(:, 2), 'LineWidth', 2)
plot(c/d, a/b, 'r*')
text(c/d + 0.6, a/b + 0.4, 'equilibrium (c/d, a/b)')
hold off; grid on
title('phase portrait'); xlabel('prey'); ylabel('predators')
