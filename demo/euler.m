% Euler's formula and the complex plane: e^(i*theta) = cos(theta) + i*sin(theta)
t = linspace(0, 2*pi, 400);
z = exp(1i*t);
th = pi/3;  w = exp(1i*th);

subplot(2, 2, 1)
plot(real(z), imag(z)); hold on
plot([0 real(w)], [0 imag(w)], 'r', 'LineWidth', 2)
plot([real(w) real(w)], [0 imag(w)], 'k--', [0 real(w)], [0 0], 'k--')
plot(real(w), imag(w), 'ro')
text(real(w) + 0.08, imag(w) + 0.08, 'e^{i\theta}, \theta = \pi/3')
text(-1.28, -0.93, 'e^{i\theta} = cos\theta + i sin\theta')
hold off; axis equal; grid on
title('Euler circle'); xlabel('Re'); ylabel('Im')

% the 7th roots of unity and a decaying spiral e^{(-0.15 + i) t}
subplot(2, 2, 2)
k = 0:7;  r = exp(2i*pi*k/7);   % k = 7 closes the polygon
s = exp((-0.15 + 1i) * linspace(0, 6*pi, 600));
plot(real(s), imag(s)); hold on
plot(real(r), imag(r), 'o-')
hold off; axis equal; grid on
title('z^7 = 1 and e^{(-0.15+i)t}'); xlabel('Re'); ylabel('Im')

% e^{it} as a helix: rotation in the complex plane while t moves on
subplot(2, 2, 3)
T = linspace(0, 6*pi, 600);
plot3(cos(T), sin(T), T); grid on
title('e^{it} over t'); xlabel('Re'); ylabel('Im'); zlabel('t')

% |f(z)| over the complex plane: zeros of z^3 - 1, poles at ±i
subplot(2, 2, 4)
[X, Y] = meshgrid(linspace(-2, 2, 70));
Z = X + 1i*Y;
F = log10(abs((Z.^3 - 1) ./ (Z.^2 + 1)));
surf(X, Y, F); colormap('viridis')
title('log_{10}|(z^3 - 1)/(z^2 + 1)|'); xlabel('Re'); ylabel('Im')
